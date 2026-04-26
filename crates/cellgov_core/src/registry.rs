//! Unit registry: assigns stable `UnitId`s, stores units behind an
//! object-safe trait, iterates in id order.
//!
//! [`ExecutionUnit`] has an associated `Snapshot` type so `dyn ExecutionUnit`
//! is not object-safe; [`RegisteredUnit`] mirrors the runtime-visible
//! methods and is blanket-impl'd for every `U: ExecutionUnit + 'static`.
//! Snapshots live outside this trait so concrete unit types pick their
//! own representation.

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, UnitStatus};
use cellgov_time::Budget;
use std::collections::BTreeMap;

/// Object-safe view of an execution unit.
///
/// Blanket-impl'd for every `U: ExecutionUnit + 'static`. Method
/// contracts mirror [`ExecutionUnit`]; see that trait for the
/// authoritative docs.
pub trait RegisteredUnit: 'static {
    /// Stable id assigned at registration.
    fn unit_id(&self) -> UnitId;

    /// Coarse runnability state queried by the scheduler.
    fn status(&self) -> UnitStatus;

    /// Run the unit until it yields.
    fn run_until_yield(
        &mut self,
        budget: Budget,
        ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult;

    /// Drain per-instruction state fingerprints retired during the most
    /// recent `run_until_yield`. Default empty (observability only).
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        Vec::new()
    }

    /// Drain full-register snapshots collected inside the zoom-in window.
    /// Default empty (observability only).
    fn drain_retired_state_full(&mut self) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)> {
        Vec::new()
    }

    /// Drain instruction-variant profiling data. Default empty.
    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        Vec::new()
    }

    /// Drain adjacent-pair profiling data. Default empty.
    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        Vec::new()
    }

    /// Notify the unit that guest memory in `[addr, addr+len)` was written.
    ///
    /// Any unit that caches decoded instructions, a translation block
    /// index, a shadow PC ring, or anything else derived from guest code
    /// MUST override. PPU/SPU units pick this up via the blanket impl;
    /// the default no-op is correct only for units that derive nothing
    /// from guest code.
    fn invalidate_code(&mut self, _addr: u64, _len: u64) {}

    /// Shadow hit/miss counters. Default `(0, 0)` (diagnostic only).
    fn shadow_stats(&self) -> (u64, u64) {
        (0, 0)
    }
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
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionUnit::run_until_yield(self, budget, ctx, effects)
    }

    #[inline]
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        ExecutionUnit::drain_retired_state_hashes(self)
    }

    #[inline]
    fn drain_retired_state_full(&mut self) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)> {
        ExecutionUnit::drain_retired_state_full(self)
    }

    #[inline]
    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        ExecutionUnit::drain_profile_insns(self)
    }

    #[inline]
    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        ExecutionUnit::drain_profile_pairs(self)
    }

    #[inline]
    fn invalidate_code(&mut self, addr: u64, len: u64) {
        ExecutionUnit::invalidate_code(self, addr, len)
    }

    #[inline]
    fn shadow_stats(&self) -> (u64, u64) {
        ExecutionUnit::shadow_stats(self)
    }
}

/// The runtime's unit registry.
///
/// `UnitId`s come from a monotonic counter; stable across runs when
/// registration order is deterministic. `BTreeMap` keying guarantees
/// id-ordered iteration independent of insertion order.
#[derive(Default)]
pub struct UnitRegistry {
    next_id: u64,
    units: BTreeMap<UnitId, Box<dyn RegisteredUnit>>,
    /// Runtime-side status overrides. Written by the commit pipeline,
    /// cleared when the unit next runs. Takes precedence over the
    /// unit's self-reported `status()` for scheduling and hashing.
    status_overrides: BTreeMap<UnitId, UnitStatus>,
    /// Per-unit pending `MailboxReceiveAttempt` pops, drained into
    /// `ExecutionContext::received_messages` at next step.
    pending_receives: BTreeMap<UnitId, Vec<u32>>,
    /// Per-unit pending syscall return code, drained into
    /// `ExecutionContext::syscall_return` at next step.
    pending_syscall_returns: BTreeMap<UnitId, u64>,
    /// Per-unit register writes injected by HLE dispatch; drained
    /// alongside syscall returns.
    pending_register_writes: BTreeMap<UnitId, Vec<(u8, u64)>>,
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

    /// Register a unit; `factory` receives the allocated id and must
    /// return a unit whose `unit_id()` equals that id.
    ///
    /// # Panics
    ///
    /// Panics if the constructed unit's `unit_id()` disagrees with the
    /// assigned id.
    ///
    /// `next_id` only advances on successful construction; a factory
    /// panic leaves the counter untouched so a caller that retries
    /// after catching the unwind reuses the same id. A hole in the id
    /// sequence would silently change replay hashes.
    pub fn register_with<U, F>(&mut self, factory: F) -> UnitId
    where
        U: ExecutionUnit + 'static,
        F: FnOnce(UnitId) -> U,
    {
        let id = UnitId::new(self.next_id);
        let unit = factory(id);
        assert_eq!(
            ExecutionUnit::unit_id(&unit),
            id,
            "registered unit reported {} but registry assigned {}",
            ExecutionUnit::unit_id(&unit).raw(),
            id.raw(),
        );
        self.next_id += 1;
        let prev = self.units.insert(id, Box::new(unit));
        debug_assert!(
            prev.is_none(),
            "UnitRegistry: next_id {id:?} already had a unit -- monotonic counter wrapped or a \
             future refactor started recycling ids; duplicate insert would silently drop the \
             old unit"
        );
        id
    }

    /// Register a unit produced by a boxed factory. Same id-allocation
    /// and factory-panic contract as [`Self::register_with`].
    pub fn register_dynamic(
        &mut self,
        factory: &dyn Fn(UnitId) -> Box<dyn RegisteredUnit>,
    ) -> UnitId {
        let id = UnitId::new(self.next_id);
        let unit = factory(id);
        assert_eq!(
            unit.unit_id(),
            id,
            "registered unit reported {} but registry assigned {}",
            unit.unit_id().raw(),
            id.raw(),
        );
        self.next_id += 1;
        let prev = self.units.insert(id, unit);
        debug_assert!(
            prev.is_none(),
            "UnitRegistry: next_id {id:?} already had a unit -- monotonic counter wrapped or a \
             future refactor started recycling ids; duplicate insert would silently drop the \
             old unit"
        );
        id
    }
}

impl UnitRegistry {
    /// Borrow a unit by id, if present.
    #[inline]
    pub fn get(&self, id: UnitId) -> Option<&dyn RegisteredUnit> {
        self.units.get(&id).map(|u| u.as_ref())
    }

    /// Mutably borrow a unit by id, if present.
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

    /// Iterate unit ids whose effective status is `Runnable`.
    pub fn runnable_ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.ids()
            .filter(move |id| self.effective_status(*id) == Some(UnitStatus::Runnable))
    }

    /// Count the units whose effective status is `Runnable`.
    ///
    /// Respects status overrides. The scheduler uses the count to
    /// short-circuit `AllBlocked` (zero) and single-runnable (one)
    /// cases without walking the rotation.
    pub fn count_runnable(&self) -> usize {
        if self.status_overrides.is_empty() {
            return self
                .units
                .values()
                .filter(|u| u.status() == UnitStatus::Runnable)
                .count();
        }
        self.runnable_ids().count()
    }
}

impl UnitRegistry {
    /// Effective status of a unit: runtime override if set, else the
    /// unit's self-reported `status()`.
    pub fn effective_status(&self, id: UnitId) -> Option<UnitStatus> {
        let unit = self.units.get(&id)?;
        Some(
            self.status_overrides
                .get(&id)
                .copied()
                .unwrap_or_else(|| unit.status()),
        )
    }

    /// Set a runtime-side status override. No-op for unknown ids.
    pub fn set_status_override(&mut self, id: UnitId, status: UnitStatus) {
        if self.units.contains_key(&id) {
            self.status_overrides.insert(id, status);
        }
    }

    /// Clear a runtime-side status override, if any. Called every step;
    /// the `is_empty()` guard avoids a `BTreeMap::remove` probe in the
    /// common case of no overrides.
    pub fn clear_status_override(&mut self, id: UnitId) {
        if self.status_overrides.is_empty() {
            return;
        }
        self.status_overrides.remove(&id);
    }
}

impl UnitRegistry {
    /// Push a received mailbox message into the unit's inbox.
    ///
    /// Silently drops writes to unknown ids (debug-assert first) to
    /// keep the pending map registry-consistent.
    pub fn push_receive(&mut self, id: UnitId, message: u32) {
        if !self.units.contains_key(&id) {
            debug_assert!(
                false,
                "push_receive for unknown UnitId {id:?} (would leak into pending_receives)"
            );
            return;
        }
        self.pending_receives.entry(id).or_default().push(message);
    }

    /// Drain all pending receives for `id` in push order.
    ///
    /// The `is_empty()` guard short-circuits the common no-pending case
    /// and assumes single-threaded access (guaranteed by `&mut self`
    /// today); remove the guard if this method moves behind a lock.
    #[inline]
    pub fn drain_receives(&mut self, id: UnitId) -> Vec<u32> {
        if self.pending_receives.is_empty() {
            return Vec::new();
        }
        self.pending_receives.remove(&id).unwrap_or_default()
    }

    /// Store a syscall return code for `id`. Unknown-id policy matches
    /// [`Self::push_receive`].
    pub fn set_syscall_return(&mut self, id: UnitId, code: u64) {
        if !self.units.contains_key(&id) {
            debug_assert!(
                false,
                "set_syscall_return for unknown UnitId {id:?} \
                 (would leak into pending_syscall_returns)"
            );
            return;
        }
        self.pending_syscall_returns.insert(id, code);
    }

    /// Drain the pending syscall return for `id`. Guard behaviour per
    /// [`Self::drain_receives`].
    #[inline]
    pub fn drain_syscall_return(&mut self, id: UnitId) -> Option<u64> {
        if self.pending_syscall_returns.is_empty() {
            return None;
        }
        self.pending_syscall_returns.remove(&id)
    }

    /// Queue a register write for the next step of `id`. Unknown-id
    /// policy matches [`Self::push_receive`].
    pub fn push_register_write(&mut self, id: UnitId, reg: u8, value: u64) {
        if !self.units.contains_key(&id) {
            debug_assert!(
                false,
                "push_register_write for unknown UnitId {id:?} \
                 (would leak into pending_register_writes)"
            );
            return;
        }
        self.pending_register_writes
            .entry(id)
            .or_default()
            .push((reg, value));
    }

    /// Drain pending register writes for `id`. Guard behaviour per
    /// [`Self::drain_receives`].
    #[inline]
    pub fn drain_register_writes(&mut self, id: UnitId) -> Vec<(u8, u64)> {
        if self.pending_register_writes.is_empty() {
            return Vec::new();
        }
        self.pending_register_writes.remove(&id).unwrap_or_default()
    }
}

impl UnitRegistry {
    /// FNV-1a over the `id.raw()` LE bytes of every runnable unit, in
    /// id order. Empty set hashes to the FNV-1a empty-input value.
    ///
    /// Wire-format contract: pinned by `runnable_queue_hash_wire_format_golden`;
    /// any drift invalidates every existing trace.
    pub fn runnable_queue_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for id in self.units.keys() {
            if self.effective_status(*id) == Some(UnitStatus::Runnable) {
                hasher.write(&id.raw().to_le_bytes());
            }
        }
        hasher.finish()
    }

    /// FNV-1a over (`id.raw()` LE, `status_byte(status)`) for every unit
    /// in id order. Uses effective status so overrides are hashed.
    ///
    /// Wire-format contract: pinned by `status_hash_wire_format_golden`.
    /// [`status_byte`] is the explicit mapping (not `as u8`) so a future
    /// `#[repr]` change cannot silently drift the hash.
    pub fn status_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for (id, unit) in self.units.iter() {
            hasher.write(&id.raw().to_le_bytes());
            let status = self
                .status_overrides
                .get(id)
                .copied()
                .unwrap_or_else(|| unit.status());
            hasher.write(&[status_byte(status)]);
        }
        hasher.finish()
    }
}

/// Explicit `UnitStatus -> u8` mapping for [`UnitRegistry::status_hash`].
///
/// Exhaustive (no `_ =>`): adding a `UnitStatus` variant without updating
/// this is a compile error, not a silent hash drift.
fn status_byte(status: UnitStatus) -> u8 {
    match status {
        UnitStatus::Runnable => 0,
        UnitStatus::Blocked => 1,
        UnitStatus::Faulted => 2,
        UnitStatus::Finished => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_exec::{LocalDiagnostics, YieldReason};
    use cellgov_mem::GuestMemory;
    use cellgov_time::InstructionCost;

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
            effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            self.steps += 1;
            effects.push(Effect::TraceMarker {
                marker: self.steps as u32,
                source: self.id,
            });
            ExecutionStepResult {
                yield_reason: YieldReason::BudgetExhausted,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
            }
        }

        fn snapshot(&self) -> u64 {
            self.steps
        }
    }

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
            _effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
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
        let mut effects = Vec::new();
        let step = u.run_until_yield(Budget::new(5), &ctx, &mut effects);
        assert_eq!(step.consumed_cost, InstructionCost::new(5));
        assert_eq!(effects.len(), 1);
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
        let mut effects = Vec::new();
        for (_, u) in r.iter_mut() {
            effects.clear();
            u.run_until_yield(Budget::new(1), &ctx, &mut effects);
            total += effects.len();
        }
        assert_eq!(total, 3);
    }

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
            _effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_cost: InstructionCost::new(budget.raw()),
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
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
    fn count_runnable_matches_runnable_ids() {
        let mut r = UnitRegistry::new();
        let (h0, f0) = status_unit(UnitStatus::Runnable);
        let (h1, f1) = status_unit(UnitStatus::Blocked);
        let (h2, f2) = status_unit(UnitStatus::Runnable);
        r.register_with(f0);
        r.register_with(f1);
        r.register_with(f2);
        assert_eq!(r.count_runnable(), 2);
        assert_eq!(r.runnable_ids().count(), 2);
        r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
        assert_eq!(r.count_runnable(), 1);
        h1.set(UnitStatus::Runnable);
        assert_eq!(r.count_runnable(), 2);
        r.clear_status_override(UnitId::new(0));
        assert_eq!(r.count_runnable(), 3);
        let _ = (h0, h2);
    }

    #[test]
    fn count_runnable_empty_registry_is_zero() {
        let r = UnitRegistry::new();
        assert_eq!(r.count_runnable(), 0);
    }

    #[test]
    fn count_runnable_all_blocked_is_zero() {
        let mut r = UnitRegistry::new();
        for _ in 0..3 {
            let (_h, f) = status_unit(UnitStatus::Blocked);
            r.register_with(f);
        }
        assert_eq!(r.count_runnable(), 0);
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
        r.register_with(|_assigned| LyingUnit);
    }

    /// Pins the `status_hash` wire format; catches reorders within
    /// [`super::status_byte`] that the exhaustive match cannot.
    #[test]
    fn status_hash_wire_format_golden() {
        let mut r = UnitRegistry::new();
        let (_h0, f0) = status_unit(UnitStatus::Runnable);
        let (_h1, f1) = status_unit(UnitStatus::Blocked);
        let (_h2, f2) = status_unit(UnitStatus::Finished);
        r.register_with(f0);
        r.register_with(f1);
        r.register_with(f2);
        const EXPECTED_STATUS_HASH: u64 = 0xE465_5B46_398E_DE44;
        assert_eq!(
            r.status_hash(),
            EXPECTED_STATUS_HASH,
            "status_hash wire format drifted; if this change was \
             intentional, every existing trace is now incompatible"
        );
    }

    /// Pins the `runnable_queue_hash` wire format; catches drift in the
    /// runnable-predicate shape that `status_byte` cannot.
    #[test]
    fn runnable_queue_hash_wire_format_golden() {
        let mut r = UnitRegistry::new();
        let (_h0, f0) = status_unit(UnitStatus::Runnable);
        let (_h1, f1) = status_unit(UnitStatus::Blocked);
        let (_h2, f2) = status_unit(UnitStatus::Runnable);
        let (_h3, f3) = status_unit(UnitStatus::Finished);
        r.register_with(f0);
        r.register_with(f1);
        r.register_with(f2);
        r.register_with(f3);
        const EXPECTED_RUNNABLE_QUEUE_HASH: u64 = 0xC615_ADCB_76DD_F8A7;
        assert_eq!(
            r.runnable_queue_hash(),
            EXPECTED_RUNNABLE_QUEUE_HASH,
            "runnable_queue_hash wire format drifted; if this change \
             was intentional, every existing trace is now incompatible"
        );
    }

    /// AssertUnwindSafe holds only while `register_with` performs no
    /// `&mut self` mutation before `factory(id)` returns. Adding any
    /// such mutation before the factory call regresses this test's
    /// soundness silently.
    #[test]
    fn factory_panic_does_not_burn_next_id() {
        let mut r = UnitRegistry::new();

        let id0 = r.register_with(|id| CountingUnit { id, steps: 0 });
        assert_eq!(id0, UnitId::new(0));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            r.register_with::<CountingUnit, _>(|_id| panic!("synthetic factory failure"));
        }));
        assert!(result.is_err(), "factory must have panicked");

        let id1 = r.register_with(|id| CountingUnit { id, steps: 0 });
        assert_eq!(
            id1,
            UnitId::new(1),
            "next_id must not advance when a factory panics -- \
             a hole in the id sequence silently changes replay hashes"
        );
        assert_eq!(r.len(), 2);
    }
}
