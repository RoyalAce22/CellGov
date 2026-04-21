//! Unit registry seam.
//!
//! Units are not free-floating: they are created via a registry call
//! that assigns a stable `UnitId`, records the constructor parameters
//! in the trace, and makes the unit known to the scheduler. Dynamic
//! spawning (for example, SPU thread group creation via
//! `register_dynamic`) allocates ids from the same monotonic counter
//! used by the static-registration path.
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
//! Snapshots are not part of `RegisteredUnit`. The trait exposes
//! `drain_retired_state_hashes` and `drain_retired_state_full` for the
//! runtime's per-step fingerprint and zoom-in windows; full snapshot
//! serialization is handled outside the object-safe trait so concrete
//! unit types can pick their own snapshot representation.

use cellgov_effects::Effect;
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
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult;

    /// Drain per-instruction state fingerprints retired during the most
    /// recent `run_until_yield`. Same contract as
    /// [`ExecutionUnit::drain_retired_state_hashes`]. Default returns
    /// empty for units that do not opt in to per-step tracing
    /// (observability only -- omitting this cannot produce a
    /// correctness bug, only a gap in diagnostics).
    fn drain_retired_state_hashes(&mut self) -> Vec<(u64, u64)> {
        Vec::new()
    }

    /// Drain full-register snapshots collected inside the unit's
    /// configured zoom-in window. Same contract as
    /// [`ExecutionUnit::drain_retired_state_full`]. Default empty
    /// (observability only -- same rationale as
    /// [`Self::drain_retired_state_hashes`]).
    fn drain_retired_state_full(&mut self) -> Vec<(u64, [u64; 32], u64, u64, u64, u32)> {
        Vec::new()
    }

    /// Drain instruction-variant profiling data. Default empty
    /// (profiling is off-path; omitting the override only drops
    /// the profile, never changes unit semantics).
    fn drain_profile_insns(&mut self) -> Vec<(&'static str, u64)> {
        Vec::new()
    }

    /// Drain adjacent-pair profiling data. Default empty (same
    /// off-path rationale as [`Self::drain_profile_insns`]).
    fn drain_profile_pairs(&mut self) -> Vec<((&'static str, &'static str), u64)> {
        Vec::new()
    }

    /// Notify the unit that guest memory in `[addr, addr+len)` was
    /// written. Same contract as [`ExecutionUnit::invalidate_code`].
    ///
    /// ## OVERRIDE THIS IF YOUR UNIT CACHES TRANSLATIONS
    ///
    /// The default is a silent no-op and is a correctness footgun,
    /// not a performance one. Any unit that caches decoded
    /// instructions, a translation block index, a shadow PC ring,
    /// or any other data derived from guest code will silently
    /// execute stale code after a guest memory write if it
    /// inherits the default. PPU and SPU execution units already
    /// override this through the `ExecutionUnit` blanket impl; a
    /// direct `RegisteredUnit` impl that represents an executing
    /// unit MUST override. Use this default only for units that
    /// demonstrably derive nothing from guest code (e.g. synthetic
    /// test harnesses, pure event sinks).
    fn invalidate_code(&mut self, _addr: u64, _len: u64) {}

    /// Shadow hit/miss counters. Same contract as
    /// [`ExecutionUnit::shadow_stats`]. Default `(0, 0)` (diagnostic
    /// only; off-path like the profile methods above).
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
    /// Per-unit syscall return code. Set by the runtime after
    /// dispatching a `YieldReason::Syscall` through the LV2 host.
    /// Drained at the start of the next `run_until_yield` and passed
    /// to the unit via `ExecutionContext::syscall_return`.
    pending_syscall_returns: BTreeMap<UnitId, u64>,
    /// Per-unit register writes injected by HLE dispatch (e.g., r13
    /// for TLS initialization). Drained alongside syscall returns.
    pending_register_writes: BTreeMap<UnitId, Vec<(u8, u64)>>,
}

// --- construction, size, registration ---
//
// Methods that produce a registry, query its cardinality, or add
// new units. Registration assigns monotonic ids and is the sole
// writer of `self.next_id` and `self.units` (keys). Factory-panic
// safety (see `register_with` doc) is a property of this block in
// isolation -- reorder carefully.
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
    ///
    /// `next_id` is only bumped after the factory returns successfully
    /// and the id-consistency assertion passes. A factory that panics
    /// (e.g. a guest-image parser blowing up inside an SPU thread-group
    /// constructor) leaves `next_id` untouched, so a caller that
    /// catches the unwind retries with the same id rather than
    /// punching a permanent hole in the id sequence. A hole would
    /// silently change `runnable_queue_hash` / `status_hash` across
    /// an otherwise-identical replay, breaking the determinism
    /// claims the module docs make.
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

    /// Register a unit produced by a boxed factory. The runtime calls
    /// this when handling `Lv2Dispatch::RegisterSpu` -- the factory
    /// receives the freshly allocated `UnitId` and returns a boxed
    /// unit. Same monotonic id allocation (and same factory-panic
    /// safety) as `register_with`.
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

// --- access and iteration ---
//
// Read-paths over the registered unit set. The scheduler lives
// against this block: `get`/`get_mut` for targeted drives,
// `iter`/`iter_mut` for the round-robin walk, and the runnable-id
// helpers for the fast-path scheduling decision. Methods here
// must stay side-effect free from the caller's perspective --
// callers read through this surface every step of the hot loop.
impl UnitRegistry {
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

    /// Iterate unit ids whose effective status is `Runnable`.
    pub fn runnable_ids(&self) -> impl Iterator<Item = UnitId> + '_ {
        self.ids()
            .filter(move |id| self.effective_status(*id) == Some(UnitStatus::Runnable))
    }

    /// Count the units whose effective status is `Runnable`.
    ///
    /// Used by the scheduler's multi-unit fast path: when the count
    /// is zero the runtime can yield `AllBlocked` without walking the
    /// full iteration order, and when the count is one the scheduler
    /// can skip the two-pass rotation and pick the single runnable
    /// unit directly. The count reflects `effective_status`, so
    /// runtime overrides (e.g. a unit parked on a PPU thread join)
    /// are honored.
    ///
    /// The `status_overrides.is_empty()` branch takes the same
    /// shape as the guard pattern in [`Self::drain_receives`] and
    /// [`Self::clear_status_override`]: skip the per-id probe
    /// against an empty override map and query `unit.status()`
    /// through the registered-unit vtable directly. Whether this
    /// actually measures faster than the unguarded path
    /// (`BTreeMap::get` on an empty map is nearly free; the vtable
    /// dispatch is not) has not been benchmarked -- treat this as
    /// a shape-consistency change matching the other fast paths
    /// rather than a measured hot-loop optimization. If a future
    /// profile shows the unguarded form is just as fast, collapse
    /// both branches into the slow path.
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

// --- status overrides ---
//
// Runtime-side overrides layered on top of the unit's self-reported
// `status()`. The commit pipeline is the sole writer. `effective_status`
// is the unified reader consumed by scheduler (via `runnable_ids`)
// and hash routines. Overrides are expected to live for one step --
// `Runtime::step` clears before running the overridden unit.
impl UnitRegistry {
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
    /// self-reported `status()` again. Fast path: `Runtime::step`
    /// calls this every step, but the override map is empty on
    /// every step that is not a wake boundary -- short-circuiting
    /// on `is_empty()` saves a BTreeMap probe in the PPU-bound
    /// hot loop.
    pub fn clear_status_override(&mut self, id: UnitId) {
        if self.status_overrides.is_empty() {
            return;
        }
        self.status_overrides.remove(&id);
    }
}

// --- pending state (receives / syscall returns / register writes) ---
//
// Three parallel per-unit inboxes fed by the commit pipeline and
// drained by the runtime at the start of each `run_until_yield`.
// All three share the same footgun surface (id validation on
// write, fast-path empty-map guard on drain, single-threaded
// assumption on the guard); the shape is deliberately parallel so
// that adding a fourth inbox stays a mechanical copy of the
// existing ones rather than a design decision.
impl UnitRegistry {
    /// Push a received message into a unit's per-unit inbox. The
    /// commit pipeline calls this when `MailboxReceiveAttempt`
    /// successfully pops a message from a non-empty mailbox.
    ///
    /// Silently no-ops when `id` does not name a registered unit.
    /// Mirrors [`Self::set_status_override`]'s policy: a stray
    /// write (off-by-one in the commit pipeline, stale cross-crate
    /// id, unit never actually registered) would otherwise land in
    /// `pending_receives`, never be consumed, and grow the map
    /// unbounded with no diagnostic. Debug builds panic so the bug
    /// surfaces in tests; release no-ops to match the rest of the
    /// pending-state API.
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

    /// Drain all pending received messages for `id`, returning them
    /// in the order they were pushed. Returns an empty vec if there
    /// are no pending messages. The runtime calls this at the start
    /// of each `run_until_yield` to build the `ExecutionContext`.
    ///
    /// Fast path: the PPU-driven hot loop hits an empty map on every
    /// step (no pending HLE-driven receive). `BTreeMap::remove`
    /// probes the root even when the map is empty; guarding on
    /// `is_empty()` costs one field load per step and short-circuits
    /// the probe, which is measurable across hundreds of millions of
    /// steps.
    ///
    /// ## Thread-safety note
    ///
    /// The `is_empty()` short-circuit assumes single-threaded access
    /// to the registry (which `&mut self` already enforces at the
    /// type level). A future refactor that introduces per-unit
    /// parallel draining (rayon over units, say) would break
    /// subtly: the short-circuit does not commute with a concurrent
    /// `push_receive` landing on a different id. The fast-path
    /// guard here is correct for the current caller and silently
    /// fragile for any other; if this method ever moves behind a
    /// lock or a concurrent iterator, remove the guard.
    #[inline]
    pub fn drain_receives(&mut self, id: UnitId) -> Vec<u32> {
        if self.pending_receives.is_empty() {
            return Vec::new();
        }
        self.pending_receives.remove(&id).unwrap_or_default()
    }

    /// Store a syscall return code for `id`. The runtime calls this
    /// after `Lv2Host::dispatch` returns `Immediate { code, .. }` so
    /// the unit can read the code on its next step.
    ///
    /// Silently no-ops for unknown ids with a debug_assert. See
    /// [`Self::push_receive`] for the full rationale.
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

    /// Drain the pending syscall return for `id`, if any. The runtime
    /// calls this at the start of each `run_until_yield` to build the
    /// `ExecutionContext`. Fast path guard and thread-safety note:
    /// see [`UnitRegistry::drain_receives`].
    #[inline]
    pub fn drain_syscall_return(&mut self, id: UnitId) -> Option<u64> {
        if self.pending_syscall_returns.is_empty() {
            return None;
        }
        self.pending_syscall_returns.remove(&id)
    }

    /// Queue a register write for the next run_until_yield of `id`.
    ///
    /// Silently no-ops for unknown ids with a debug_assert. See
    /// [`Self::push_receive`] for the full rationale.
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

    /// Drain pending register writes for `id`. Fast path guard and
    /// thread-safety note: see [`UnitRegistry::drain_receives`].
    #[inline]
    pub fn drain_register_writes(&mut self, id: UnitId) -> Vec<(u8, u64)> {
        if self.pending_register_writes.is_empty() {
            return Vec::new();
        }
        self.pending_register_writes.remove(&id).unwrap_or_default()
    }
}

// --- checkpoint hashes ---
//
// Wire-format-sensitive hash outputs consumed by the trace/replay
// layer. Changes to the byte order, the status-byte mapping
// (`status_byte`), or the set of fields hashed will invalidate
// every existing trace. Golden tests in the `tests` module below
// pin both outputs; update them in the same commit as any hash
// change so the trace-incompatibility window is visible in git
// history.
impl UnitRegistry {
    /// 64-bit deterministic hash of the ordered set of unit ids whose
    /// effective status is `Runnable`.
    ///
    /// Used as the `RunnableQueue` checkpoint hash. FNV-1a over the `id.raw()`
    /// le-bytes of each runnable unit, in id order. The empty set
    /// (no runnable units) hashes to the FNV-1a empty-input value.
    pub fn runnable_queue_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        for id in self.units.keys() {
            if self.effective_status(*id) == Some(UnitStatus::Runnable) {
                hasher.write(&id.raw().to_le_bytes());
            }
        }
        hasher.finish()
    }

    /// 64-bit deterministic hash of every unit's (id, effective status)
    /// pair in id order.
    ///
    /// Used as the `UnitStatus` checkpoint hash. FNV-1a, no host-time
    /// inputs, no external deps. Walks the underlying [`BTreeMap`]
    /// in id order so the result is independent of registration
    /// history.
    ///
    /// ## Wire-format contract
    ///
    /// The per-status byte is explicitly mapped by `status_byte`
    /// rather than using `UnitStatus as u8`. The enum's `#[repr(u8)]`
    /// plus discriminants-locked comment already pin the values, but
    /// relying on an implicit cast means a future accidental removal
    /// of `#[repr(u8)]` or a `#[repr(Rust)]` refactor would silently
    /// change this hash and break replay compatibility with every
    /// existing trace. The explicit mapping makes the wire contract
    /// readable in this file and fails loudly on reorder.
    ///
    /// Replay tooling compares pairs of these values to assert that
    /// two runs reached the same set of unit statuses. The empty
    /// registry hashes to the FNV-1a empty-input value, which the
    /// runtime trace records on its first checkpoint.
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

/// Explicit `UnitStatus -> u8` mapping used by [`UnitRegistry::status_hash`].
///
/// Values match the discriminants declared on `UnitStatus` today, so
/// replacing the previous `as u8` cast with this function produces
/// byte-for-byte identical hashes for every existing trace. The
/// mapping is exhaustive (no `_ =>`), so adding a variant to
/// `UnitStatus` without updating this mapping is a compile error --
/// which is the whole point of making the wire format explicit.
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
            effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            self.steps += 1;
            effects.push(Effect::TraceMarker {
                marker: self.steps as u32,
                source: self.id,
            });
            ExecutionStepResult {
                yield_reason: YieldReason::BudgetExhausted,
                consumed_budget: budget,
                local_diagnostics: LocalDiagnostics::empty(),
                fault: None,
                syscall_args: None,
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
            _effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
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
        assert_eq!(step.consumed_budget, Budget::new(5));
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
            _effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            ExecutionStepResult {
                yield_reason: YieldReason::Finished,
                consumed_budget: budget,
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
        // Flip unit 0 to Blocked via override -- count drops.
        r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
        assert_eq!(r.count_runnable(), 1);
        // Flip unit 1 from Blocked to Runnable self-report.
        h1.set(UnitStatus::Runnable);
        assert_eq!(r.count_runnable(), 2);
        // Clear override on unit 0.
        r.clear_status_override(UnitId::new(0));
        assert_eq!(r.count_runnable(), 3);
        // Quiet unused warnings.
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

    /// Golden test for the [`UnitRegistry::status_hash`] wire format.
    ///
    /// The exhaustive `match` in [`super::status_byte`] catches
    /// variant *additions* at compile time (a new `UnitStatus`
    /// without an arm is an error). It does not catch someone
    /// reordering the existing arms or changing a literal (e.g.
    /// `UnitStatus::Blocked => 1` -> `UnitStatus::Blocked => 5`) --
    /// such a change is a silent hash drift that breaks replay
    /// compatibility with every existing trace.
    ///
    /// This test pins a fixed three-unit registry at a known hash.
    /// A drift in `status_byte`, in the byte-order of the FNV-1a
    /// writes, or in how effective-status is resolved will fail
    /// here with a before/after value the reader can diff. The
    /// golden value below was computed by running this same
    /// scenario once; if it ever has to change, the commit that
    /// changes it is the one that invalidates every prior trace.
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

    /// Golden test for the [`UnitRegistry::runnable_queue_hash`] wire
    /// format. Covers a second silent-drift vector that
    /// `status_byte` does not: the *predicate* used to decide which
    /// ids are hashed. `runnable_queue_hash` emits only ids whose
    /// `effective_status(*id) == Some(UnitStatus::Runnable)`; a
    /// future variant like `UnitStatus::RunnableButWaitingForFoo`
    /// that semantically belongs in the runnable set but is not
    /// literally the `Runnable` variant would silently change this
    /// hash for every existing trace with no compile-time signal.
    /// Pinning a mixed-status fixture here makes that drift loud.
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

    /// A factory that panics must not burn a UnitId. If the caller
    /// catches the unwind and retries, the new registration has to
    /// receive the same id the panicking factory saw -- otherwise
    /// the id sequence has a permanent hole and
    /// `runnable_queue_hash` / `status_hash` silently diverge from
    /// a clean replay.
    #[test]
    fn factory_panic_does_not_burn_next_id() {
        let mut r = UnitRegistry::new();

        // First, register one unit normally so the counter sits at 1.
        let id0 = r.register_with(|id| CountingUnit { id, steps: 0 });
        assert_eq!(id0, UnitId::new(0));

        // Catch_unwind around a factory that panics.
        //
        // This test's soundness under AssertUnwindSafe is not
        // automatic -- it depends on the invariant that
        // `register_with` performs NO `&mut self` mutation before
        // `factory(id)` returns. Today `register_with` only reads
        // `self.next_id` in the `UnitId::new(self.next_id)` line
        // before handing off to the factory, so the registry
        // remains in a consistent state when the factory panics
        // and subsequent reads of `r` observe valid data. If a
        // future refactor ever slips a `self.units.insert(...)`,
        // `self.pending_*` mutation, or `self.next_id += 1` in
        // before the factory call, this test will silently keep
        // passing while AssertUnwindSafe begins lying about unwind
        // safety -- and the "no burned id" guarantee the test
        // claims to pin would be gone.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            r.register_with::<CountingUnit, _>(|_id| panic!("synthetic factory failure"));
        }));
        assert!(result.is_err(), "factory must have panicked");

        // The next successful registration must pick up the id the
        // panicked factory saw. If fix #2 ever regresses to
        // incrementing next_id before the factory runs, this will
        // see id 2 instead of id 1 and fail.
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
