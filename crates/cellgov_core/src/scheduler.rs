//! Deterministic scheduler -- step 1 of the runtime pipeline.
//!
//! The runtime pipeline is:
//!
//! 1. select runnable unit deterministically
//! 2. grant budget
//! 3. run unit until yield
//! 4. ... (validation, commit, event injection, time advance, trace)
//!
//! This module owns step 1. Given a [`UnitRegistry`], it picks the next
//! [`UnitId`] to schedule based purely on the unit statuses currently
//! reported by the registry. It does not call `run_until_yield`, it
//! does not touch the commit pipeline, it does not advance time. The
//! runtime loop in [`crate::runtime`] will compose the scheduler with
//! the rest of the pipeline once the pipeline exists.
//!
//! Determinism contract: a scheduler implementation is a pure function
//! of `(its own state, registry contents)`. It must not consult host
//! time, host thread scheduling, `HashMap` iteration order, or any
//! other nondeterministic input.

use crate::registry::UnitRegistry;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;

/// Pluggable scheduling policy.
///
/// Concrete scheduler types stay private to `cellgov_core`; other
/// crates see only traits and immutable data packets. Other crates
/// plug in their own implementations (for example, the bounded
/// schedule explorer), with [`RoundRobinScheduler`] serving as the
/// default shipped alongside the trait.
pub trait Scheduler {
    /// Select the next runnable unit, or `None` if no unit is
    /// currently runnable.
    ///
    /// Implementations may mutate internal scheduler state (a cursor,
    /// a fairness counter, etc.) but must not mutate the registry.
    /// They must be deterministic: identical scheduler state plus an
    /// identical sequence of registry-status snapshots must produce
    /// an identical sequence of selections.
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId>;
}

/// A round-robin scheduler.
///
/// Walks the registry in id order, starting from the position after
/// the last selection, and returns the first unit it finds whose
/// [`UnitStatus`] is `Runnable`. Wraps around. Skips `Blocked`,
/// `Faulted`, and `Finished` units.
///
/// This is the simplest deterministic scheduler that guarantees no
/// runnable unit can starve under fixed workload: every runnable unit
/// gets a turn before any unit gets a second turn (modulo blocking
/// transitions).
///
/// ## Performance
///
/// `select_next` walks the full registry once per call to snapshot
/// the runnable set. No benchmark currently pins this cost against
/// the alternative (walk-twice-without-snapshot) shape -- the
/// snapshot form was picked for correctness (see the
/// `effective_status`-purity note inside `select_next`), not after a
/// measured win. If a future profile shows this is a hot-loop cost
/// on large-N workloads, revisit: either inline the two-pass scan
/// against `registry.iter()` directly and pin `effective_status`
/// purity via a doc contract, or keep the snapshot but amortize
/// the allocation with a scheduler-owned reusable Vec.
///
/// ## Correctness assumptions
///
/// Two invariants this scheduler trusts [`UnitRegistry`] to uphold:
///
/// 1. `registry.iter()` yields ids in ascending order. The two-pass
///    scan depends on this -- `*id > cursor` + `*id <= cursor`
///    partitions the registry only if iteration order matches id
///    order. A `HashMap`-backed registry would silently reorder
///    selections. The [`round_robin_select_next_matches_hand_expected_sequence`]
///    test pins a fixed mixed-status scenario to a hand-written
///    expected sequence, which fails loudly on any iter-order drift
///    (including the `HashMap` case the module docs call out as a
///    determinism hazard).
///
/// 2. [`UnitId`]s are monotonic and stable once assigned. The
///    scheduler stores `last_scheduled: Option<UnitId>` across
///    calls; if the registry ever compacts ids or recycles them,
///    the cursor could point at an id that now refers to a
///    different unit. No defense at this layer -- id stability is
///    the registry's contract.
#[derive(Debug, Default)]
pub struct RoundRobinScheduler {
    /// The id of the most recently selected unit, used as the cursor
    /// for the next call. `None` means no selection yet -- the next
    /// call starts at the beginning of the registry.
    last_scheduled: Option<UnitId>,
}

impl RoundRobinScheduler {
    /// Construct a fresh scheduler with no prior selection.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the id of the most recently selected unit, if any. Used
    /// by tests and trace tooling to inspect the cursor.
    ///
    /// ## Reliability caveat for trace tooling
    ///
    /// The returned id is only trustworthy while the
    /// [`UnitRegistry`]'s monotonic-stable-id contract holds (see
    /// the struct doc's correctness-assumption #2). If the
    /// registry ever removes a unit or recycles its id, this
    /// method will happily return an id that either refers to a
    /// no-longer-present unit or to a different unit than the one
    /// originally selected. The disappearance case is detected by
    /// a `debug_assert!` on the next [`Scheduler::select_next`]
    /// call; the recycling case is not detected at this layer and
    /// requires a registry-side generation token that the current
    /// API does not expose. Tooling reading this method without
    /// calling `select_next` receives no such protection either
    /// way.
    #[inline]
    pub fn last_scheduled(&self) -> Option<UnitId> {
        self.last_scheduled
    }
}

impl Scheduler for RoundRobinScheduler {
    fn select_next(&mut self, registry: &UnitRegistry) -> Option<UnitId> {
        if registry.is_empty() {
            return None;
        }
        // Contract check: if the cursor names an id, the registry
        // must still contain SOME unit at that id. The check is
        // existence-only and deliberately narrow -- it does not
        // detect id recycling, where `c` is now live but belongs
        // to a different unit than the one the cursor was set
        // from. Recycling would need a registry-side
        // generation/creation-token on entries; nothing in the
        // current API exposes one, so this assertion catches the
        // compaction-style violation (cursor id no longer present
        // at all) but silently passes through recycling. The
        // module-doc correctness-assumption #2 is still the
        // authoritative contract; this is defense-in-depth, not
        // coverage.
        if let Some(c) = self.last_scheduled {
            debug_assert!(
                registry.get(c).is_some(),
                "scheduler cursor {c:?} names an id not present in the registry \
                 (does not detect id recycling, only disappearance)"
            );
        }
        // Snapshot-once policy: collect every runnable id into a
        // small Vec in one walk, then make all scheduling
        // decisions from that Vec. The previous implementation
        // called `effective_status` twice per unit in the
        // multi-runnable case (once for the fast-path count, once
        // for each two-pass filter), which silently assumed
        // `effective_status` is pure. If a future refactor ever
        // makes that call stateful (cache-on-read, etc.), the two
        // reads could disagree and the scheduler would pick based
        // on a view inconsistent with the count it used to decide
        // which branch to take. Walking once eliminates the
        // disagreement surface entirely.
        //
        // Cost: one Vec allocation per call, bounded by the number
        // of runnable units (typically dozens on real PS3 workloads,
        // well within the bump-allocator noise). Walks the full
        // registry once, which is a minor regression for the old
        // multi-runnable-with-early-runnables fast path and a
        // minor win for the multi-runnable-with-late-runnables
        // case (no second walk in the two-pass).
        //
        // The runnables Vec inherits ascending id order from the
        // registry's BTreeMap-driven iter(); the two-pass scan
        // below relies on this, same as the module-doc's
        // correctness assumption #1.
        let runnables: Vec<UnitId> = registry
            .iter()
            .filter(|(id, _)| registry.effective_status(*id) == Some(UnitStatus::Runnable))
            .map(|(id, _)| id)
            .collect();

        // Load-bearing invariants on `runnables`:
        //
        // 1. Ascending id order. The two-pass `find(id > c)` then
        //    `find(id <= c)` below is only a correct round-robin
        //    rotation if the Vec is ascending. The module-doc's
        //    correctness-assumption #1 is the upstream source of
        //    this property (registry.iter() walks BTreeMap in
        //    ascending key order); the pairs-check here is a
        //    defense-in-depth canary against a future registry
        //    change that silently swaps in priority-bucketed or
        //    insertion-order iteration. `windows(2)` is the
        //    portable form; `slice::is_sorted` would read better
        //    but is not stable for arbitrary `Ord` without the
        //    nightly `is_sorted` feature.
        // 2. Bounded size. Real PS3 workloads have dozens of
        //    units; a runnables snapshot with tens of thousands
        //    of entries means the registry itself is broken
        //    (runaway id allocation, a registration loop, id
        //    recycling gone wrong). Parallel to
        //    `MAX_PENDING_RESPONSES` in `SyscallResponseTable`;
        //    debug-only to avoid burdening shipping boots with a
        //    check for a bug we have not yet observed.
        debug_assert!(
            runnables.windows(2).all(|w| w[0] < w[1]),
            "scheduler runnables snapshot is not ascending: {runnables:?}"
        );
        debug_assert!(
            runnables.len() < 65_536,
            "scheduler runnables snapshot exceeded 65536; registry is likely broken"
        );

        let chosen = match runnables.len() {
            0 => None,
            1 => Some(runnables[0]),
            _ => {
                // Two-pass scan on the snapshot. First pass: ids
                // strictly after the cursor (or any id if cursor
                // is None -- the fresh-scheduler case, which
                // always takes the first runnable). Wrap pass:
                // ids <= cursor, only reachable when cursor is
                // Some AND the first pass found nothing. The
                // previous implementation had a three-way match
                // with an "unreachable in practice" None arm that
                // silently hid the logical contradiction of
                // "multiple runnables exist, cursor is None, but
                // no id was found"; the snapshot-based form
                // retires that arm entirely -- with `runnables`
                // in hand, "cursor is None and runnables non-empty"
                // simply returns `runnables[0]`.
                match self.last_scheduled {
                    Some(c) => runnables
                        .iter()
                        .copied()
                        .find(|&id| id > c)
                        .or_else(|| runnables.iter().copied().find(|&id| id <= c)),
                    None => Some(runnables[0]),
                }
            }
        };

        if let Some(id) = chosen {
            self.last_scheduled = Some(id);
        }
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_effects::Effect;
    use cellgov_exec::{
        ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, YieldReason,
    };
    use cellgov_time::Budget;
    use std::cell::Cell;

    // Local test doubles -- cellgov_testkit depends on cellgov_core,
    // so a reverse dev-dependency would create a cycle.

    /// A test unit whose status is configurable per-test. Uses interior
    /// mutability so tests can flip the status without re-registering.
    struct TestUnit {
        id: UnitId,
        status: Cell<UnitStatus>,
    }

    impl TestUnit {
        fn new(id: UnitId, status: UnitStatus) -> Self {
            Self {
                id,
                status: Cell::new(status),
            }
        }
    }

    impl ExecutionUnit for TestUnit {
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
            effects: &mut Vec<Effect>,
        ) -> ExecutionStepResult {
            effects.push(Effect::TraceMarker {
                marker: 0,
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

        fn snapshot(&self) {}
    }

    fn registry_with(statuses: &[UnitStatus]) -> UnitRegistry {
        let mut r = UnitRegistry::new();
        for &s in statuses {
            r.register_with(|id| TestUnit::new(id, s));
        }
        r
    }

    #[test]
    fn empty_registry_yields_none() {
        let mut s = RoundRobinScheduler::new();
        let r = UnitRegistry::new();
        assert_eq!(s.select_next(&r), None);
    }

    #[test]
    fn all_blocked_yields_none() {
        let mut s = RoundRobinScheduler::new();
        let r = registry_with(&[UnitStatus::Blocked, UnitStatus::Blocked]);
        assert_eq!(s.select_next(&r), None);
    }

    /// Complement to `all_blocked_yields_none`: all-blocked with
    /// a cursor already set. The plain test only exercises the
    /// fresh-scheduler (cursor=None) path; a scheduler that has
    /// already scheduled a unit and then hits an all-blocked
    /// situation routes through the same "no runnables" branch
    /// but with a non-None cursor. Returning None here must leave
    /// the cursor untouched so that when units unblock, the
    /// rotation resumes from where it left off rather than
    /// restarting at the top.
    /// Pin the "cursor on a now-non-runnable unit, rotation
    /// continues past it correctly" invariant. The current code
    /// handles this correctly because the snapshot excludes the
    /// cursor-unit when it is not runnable and the two-pass just
    /// picks the next ascending id; there is no test that
    /// exercises the scenario end-to-end. A future refactor that
    /// tries to "validate" the cursor by repositioning it when
    /// the cursor-unit becomes non-runnable could change
    /// observable behavior and every other current test would
    /// still pass. This fills the gap.
    ///
    /// Shape: three Runnable units (0, 1, 2). Advance cursor to 1.
    /// Block unit 1 (the cursor unit). Assert next pick is 2 (the
    /// smallest id strictly greater than the cursor). Unblock 1.
    /// Assert next pick is 0 (wrap past end; cursor now at 2,
    /// then wrap to 0). Assert the following pick is 1 (rotation
    /// reaches the previously-blocked cursor unit in order).
    #[test]
    fn rotation_continues_correctly_when_cursor_unit_becomes_blocked() {
        let mut r = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.last_scheduled(), Some(UnitId::new(1)));
        // Block the cursor unit.
        r.set_status_override(UnitId::new(1), UnitStatus::Blocked);
        // First pass `id > 1`: unit 2 is next in ascending order.
        assert_eq!(s.select_next(&r), Some(UnitId::new(2)));
        // Cursor is now 2. Unblock 1.
        r.clear_status_override(UnitId::new(1));
        // First pass `id > 2`: empty. Wrap pass `id <= 2`:
        // lowest ascending -> 0.
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        // Cursor is 0. First pass `id > 0`: unit 1 (the
        // previously-blocked cursor unit, now runnable).
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    #[test]
    fn all_blocked_with_cursor_set_yields_none_and_preserves_cursor() {
        let mut r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.last_scheduled(), Some(UnitId::new(0)));
        // Now block everyone via override.
        r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
        r.set_status_override(UnitId::new(1), UnitStatus::Blocked);
        assert_eq!(s.select_next(&r), None);
        assert_eq!(
            s.last_scheduled(),
            Some(UnitId::new(0)),
            "cursor must survive an all-blocked call so rotation resumes \
             correctly once units unblock"
        );
        // Unblock unit 1, confirm rotation picks it (not unit 0,
        // which is the cursor).
        r.clear_status_override(UnitId::new(1));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    #[test]
    fn single_runnable_picks_it_repeatedly() {
        let mut s = RoundRobinScheduler::new();
        let r = registry_with(&[UnitStatus::Runnable]);
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    }

    #[test]
    fn round_robin_visits_each_runnable_in_id_order() {
        let mut s = RoundRobinScheduler::new();
        let r = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(2)));
        // Wraps.
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    #[test]
    fn skips_blocked_faulted_finished() {
        let mut s = RoundRobinScheduler::new();
        let r = registry_with(&[
            UnitStatus::Blocked,
            UnitStatus::Runnable,
            UnitStatus::Faulted,
            UnitStatus::Runnable,
            UnitStatus::Finished,
        ]);
        // First call from origin: skip 0 (blocked), pick 1.
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        // Next call from after 1: skip 2 (faulted), pick 3.
        assert_eq!(s.select_next(&r), Some(UnitId::new(3)));
        // Next call from after 3: skip 4 (finished), wrap, skip 0,
        // and pick 1 again.
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    #[test]
    fn round_robin_with_only_one_runnable_among_many() {
        // Five units, only the third is runnable. Every call returns it.
        let r = registry_with(&[
            UnitStatus::Blocked,
            UnitStatus::Blocked,
            UnitStatus::Runnable,
            UnitStatus::Blocked,
            UnitStatus::Blocked,
        ]);
        let mut s = RoundRobinScheduler::new();
        for _ in 0..5 {
            assert_eq!(s.select_next(&r), Some(UnitId::new(2)));
        }
    }

    #[test]
    fn three_runnable_units_produce_identical_selection_sequence_across_runs() {
        // Determinism canary: two independent
        // RoundRobinScheduler + UnitRegistry pairs, each holding
        // three runnable test units, must produce byte-identical
        // selection sequences over a long prefix. This is the
        // scheduler-layer determinism guard that downstream sync
        // primitives can rely on. Breaking it (via accidental
        // HashMap iteration, host-time, or unstable sort) would
        // fail here before reaching any higher-level integration
        // test.
        let r_a = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        let r_b = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        let mut s_a = RoundRobinScheduler::new();
        let mut s_b = RoundRobinScheduler::new();
        let seq_a: Vec<_> = (0..100)
            .map(|_| s_a.select_next(&r_a).unwrap().raw())
            .collect();
        let seq_b: Vec<_> = (0..100)
            .map(|_| s_b.select_next(&r_b).unwrap().raw())
            .collect();
        assert_eq!(seq_a, seq_b);
        // Shape check: round-robin cycles 0, 1, 2, 0, 1, 2, ...
        for (i, id) in seq_a.iter().enumerate() {
            assert_eq!(*id, (i % 3) as u64);
        }
    }

    #[test]
    fn single_runnable_fast_path_picks_it_in_multi_unit_registry() {
        // Runnable-count fast path regression: 3 units, only unit 1
        // is runnable. The scheduler must return unit 1 without
        // walking the full two-pass rotation.
        let r = registry_with(&[
            UnitStatus::Blocked,
            UnitStatus::Runnable,
            UnitStatus::Blocked,
        ]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.last_scheduled(), Some(UnitId::new(1)));
    }

    #[test]
    fn last_scheduled_tracks_cursor() {
        let r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
        let mut s = RoundRobinScheduler::new();
        assert_eq!(s.last_scheduled(), None);
        let _ = s.select_next(&r);
        assert_eq!(s.last_scheduled(), Some(UnitId::new(0)));
        let _ = s.select_next(&r);
        assert_eq!(s.last_scheduled(), Some(UnitId::new(1)));
    }

    #[test]
    fn status_override_blocks_a_runnable_unit() {
        // Unit 0 self-reports Runnable, but the runtime overrides it
        // to Blocked. The scheduler must skip it.
        let mut r = registry_with(&[UnitStatus::Runnable, UnitStatus::Runnable]);
        r.set_status_override(UnitId::new(0), UnitStatus::Blocked);
        let mut s = RoundRobinScheduler::new();
        // Every call should return unit 1 since unit 0 is overridden.
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        // Clear the override -- unit 0 is runnable again.
        r.clear_status_override(UnitId::new(0));
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
    }

    /// Complement to `status_override_blocks_a_runnable_unit`:
    /// overrides can also force a self-reported `Blocked` unit
    /// into the runnable set (the `WakeUnit` commit path). The
    /// prior test only exercised the Runnable -> Blocked
    /// direction; a latent bug where `effective_status` silently
    /// ignored overrides going the other way would have passed
    /// that test while producing a scheduler that never woke
    /// anything.
    #[test]
    fn status_override_wakes_a_blocked_unit() {
        let mut r = registry_with(&[UnitStatus::Blocked, UnitStatus::Blocked]);
        let mut s = RoundRobinScheduler::new();
        // Baseline: both units self-report Blocked, nothing runs.
        assert_eq!(s.select_next(&r), None);
        // Wake unit 1 via override.
        r.set_status_override(UnitId::new(1), UnitStatus::Runnable);
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        // Wake unit 0 too, then verify round-robin still works
        // with mixed self-reported / overridden statuses.
        r.set_status_override(UnitId::new(0), UnitStatus::Runnable);
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
    }

    /// Pin the observable "re-pick the only runnable, regardless of
    /// cursor position" contract in the many-units case.
    ///
    /// With the single-runnable fast path in place this test exercises
    /// the fast path, not the wrap-pass-with-cursor-on-survivor
    /// branch -- the fast path ignores the cursor entirely, so the
    /// returned unit is determined purely by "who is runnable," not
    /// by "where is the cursor." The wrap-with-cursor-on-survivor
    /// path is structurally unreachable as long as the fast path
    /// exists: reaching the two-pass scan requires at least two
    /// runnables, and "cursor on the survivor when there is only
    /// one" is a single-runnable situation by definition.
    ///
    /// The test is still worth having as a behavioral regression
    /// pin -- it asserts the contract regardless of which internal
    /// path serves it. If someone ever removes the fast path, the
    /// same sequence of calls routes through the wrap pass
    /// instead, and the assertion still holds. Fills a gap between
    /// `single_runnable_picks_it_repeatedly` (one-unit registry)
    /// and `round_robin_with_only_one_runnable_among_many` (fresh
    /// scheduler).
    #[test]
    fn cursor_advanced_past_survivor_still_re_picks_it() {
        let mut r = registry_with(&[
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
            UnitStatus::Runnable,
        ]);
        let mut s = RoundRobinScheduler::new();
        // Advance cursor to unit 3.
        assert_eq!(s.select_next(&r), Some(UnitId::new(0)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(1)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(2)));
        assert_eq!(s.select_next(&r), Some(UnitId::new(3)));
        assert_eq!(s.last_scheduled(), Some(UnitId::new(3)));
        // Block every unit except 3.
        for &i in &[0u64, 1, 2, 4] {
            r.set_status_override(UnitId::new(i), UnitStatus::Blocked);
        }
        for _ in 0..5 {
            assert_eq!(s.select_next(&r), Some(UnitId::new(3)));
        }
    }

    /// Golden hand-expected selection sequence -- the most
    /// important regression guard for this module.
    ///
    /// The existing
    /// `three_runnable_units_produce_identical_selection_sequence_across_runs`
    /// test asserts two scheduler instances agree with *each
    /// other*. Two broken schedulers agreeing is not determinism;
    /// if `UnitRegistry` ever switches to `HashMap`-backed
    /// iteration (the exact hazard the module docs call out), the
    /// cross-instance test would still pass because both
    /// schedulers would be broken in the same way.
    ///
    /// This test compares against a sequence computed by hand from
    /// the round-robin contract. It fails if:
    ///   - `registry.iter()` yields in non-ascending order
    ///     (scheduler picks "first runnable after cursor" using
    ///     iter order as a proxy for id order).
    ///   - The two-pass rotation wrap-around is broken.
    ///   - The single-runnable fast path returns the wrong unit.
    ///   - Status-skip logic misses any of Blocked/Faulted/Finished.
    ///
    /// Scenario: 5 units with statuses [Blocked, Runnable,
    /// Faulted, Runnable, Finished]. Only ids 1 and 3 are
    /// runnable. Fresh scheduler, 6 consecutive selects.
    ///
    /// Hand trace:
    ///   call 1 (cursor=None): first Runnable in ascending iter -> 1
    ///   call 2 (cursor=1):    after=Some(id>1): 2 Faulted, 3 Runnable -> 3
    ///   call 3 (cursor=3):    after=Some(id>3): 4 Finished. wrap id<=3: 0 Blocked, 1 Runnable -> 1
    ///   call 4 (cursor=1):    same shape as call 2 -> 3
    ///   call 5 (cursor=3):    same shape as call 3 -> 1
    ///   call 6 (cursor=1):    same shape as call 2 -> 3
    #[test]
    fn round_robin_select_next_matches_hand_expected_sequence() {
        let r = registry_with(&[
            UnitStatus::Blocked,
            UnitStatus::Runnable,
            UnitStatus::Faulted,
            UnitStatus::Runnable,
            UnitStatus::Finished,
        ]);
        let mut s = RoundRobinScheduler::new();
        let observed: Vec<u64> = (0..6)
            .map(|_| s.select_next(&r).expect("runnable set non-empty").raw())
            .collect();
        let expected: Vec<u64> = vec![1, 3, 1, 3, 1, 3];
        assert_eq!(
            observed, expected,
            "scheduler output drifted from the hand-expected round-robin \
             sequence; probable cause: registry.iter() is no longer ascending, \
             two-pass wrap is broken, or a status skip misfired"
        );
    }
}
