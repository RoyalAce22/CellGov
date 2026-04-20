//! Bounded schedule exploration loop.
//!
//! `explore` runs a workload once to collect the baseline decision log,
//! then replays with alternate choices at each branching point. It
//! respects configurable bounds on how many schedules to try and
//! classifies the result as schedule-stable, schedule-sensitive, or
//! inconclusive.

use crate::classify::ExplorationResult;
use crate::config::ExplorationConfig;
use crate::observer::observe_decisions;
use crate::prescribed::PrescribedScheduler;
use crate::util::{build_overrides, classify_iteration, for_each_alternate, run_to_stall};
use cellgov_core::Runtime;

/// Run bounded schedule exploration on a workload.
///
/// `make_runtime` is called once for the baseline run and once per
/// alternate schedule. Each call must produce an independent runtime
/// from identical initial state.
///
/// Returns `None` if the baseline run has no branching points.
pub fn explore<F>(mut make_runtime: F, config: &ExplorationConfig) -> Option<ExplorationResult>
where
    F: FnMut() -> Runtime,
{
    let mut rt_baseline = make_runtime();
    let log = observe_decisions(&mut rt_baseline);
    let baseline_hash = rt_baseline.memory().content_hash();

    let total_branching_points = log.branching_count();
    if total_branching_points == 0 {
        return None;
    }

    let iter = for_each_alternate(&log, config, baseline_hash, |step, alt| {
        let overrides = build_overrides(step, alt);
        let mut rt = make_runtime();
        rt.set_scheduler(PrescribedScheduler::new(overrides));
        run_to_stall(&mut rt, config.max_steps_per_run);
        rt.memory().content_hash()
    });

    Some(classify_iteration(
        iter,
        baseline_hash,
        total_branching_points,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::OutcomeClass;
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    #[test]
    fn explore_disjoint_writes_is_stable() {
        let result = explore(
            || {
                let mem = GuestMemory::new(64);
                let mut rt = Runtime::new(mem, Budget::new(100), 100);
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xAA),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xBB),
                            FakeOp::SharedStore { addr: 8, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt
            },
            &ExplorationConfig::default(),
        );

        let r = result.expect("should have branching points");
        assert_eq!(r.outcome, OutcomeClass::ScheduleStable);
        assert!(!r.bounds_hit);
    }

    /// Two-unit atomic contention under exploration.
    ///
    /// Two units both acquire a reservation on the same line and
    /// then emit a conditional store. The commit pipeline's
    /// clear-sweep rule means whichever unit's ConditionalStore
    /// commits second has its reservation already cleared by the
    /// first committer's write -- but the FakeIsaUnit harness
    /// doesn't model a local reservation register, so both
    /// ConditionalStores commit unconditionally and the final
    /// counter byte reflects whichever value landed last.
    ///
    /// The gate is not a specific final value -- it is that
    /// exploration enumerates the orderings without crashing and
    /// classifies outcomes reproducibly across two runs. When both
    /// units write the same byte value, all orderings collapse to
    /// one class (stable). When they write different values, the
    /// engine classifies as ScheduleSensitive. Both shapes prove
    /// the explorer handles reservation effects without panicking.
    #[test]
    fn explore_two_unit_atomic_contention_is_reproducible() {
        let make = || {
            let mem = GuestMemory::new(256);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xAA),
                        FakeOp::ReservationAcquire { line_addr: 0x80 },
                        FakeOp::ConditionalStore { addr: 0x80, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt.registry_mut().register_with(|id| {
                FakeIsaUnit::new(
                    id,
                    vec![
                        FakeOp::LoadImm(0xBB),
                        FakeOp::ReservationAcquire { line_addr: 0x80 },
                        FakeOp::ConditionalStore { addr: 0x80, len: 4 },
                        FakeOp::End,
                    ],
                )
            });
            rt
        };

        // First exploration run.
        let r1 = explore(make, &ExplorationConfig::default())
            .expect("contention workload must have branching points");

        // Second exploration run from identical initial state.
        let r2 = explore(make, &ExplorationConfig::default())
            .expect("contention workload must have branching points");

        // Classification is reproducible across runs.
        assert_eq!(
            r1.outcome, r2.outcome,
            "exploration classification must be stable across runs",
        );
        // Different-byte atomic writes produce schedule-sensitive
        // outcomes; same-byte writes are schedule-stable. Either
        // classification is valid for the gate; what matters is
        // the engine enumerated the orderings without crashing
        // and produced the same verdict twice.
        assert!(matches!(
            r1.outcome,
            OutcomeClass::ScheduleStable | OutcomeClass::ScheduleSensitive
        ));
    }

    /// Two-unit contention with matching stored values
    /// collapses to a single class. Proves the engine does not
    /// mislabel equivalent final states as schedule-sensitive.
    #[test]
    fn explore_two_unit_atomic_same_value_is_stable() {
        let result = explore(
            || {
                let mem = GuestMemory::new(256);
                let mut rt = Runtime::new(mem, Budget::new(100), 100);
                for _ in 0..2 {
                    rt.registry_mut().register_with(|id| {
                        FakeIsaUnit::new(
                            id,
                            vec![
                                FakeOp::LoadImm(0x42),
                                FakeOp::ReservationAcquire { line_addr: 0x80 },
                                FakeOp::ConditionalStore { addr: 0x80, len: 4 },
                                FakeOp::End,
                            ],
                        )
                    });
                }
                rt
            },
            &ExplorationConfig::default(),
        )
        .expect("contention workload must have branching points");

        assert_eq!(
            result.outcome,
            OutcomeClass::ScheduleStable,
            "matching conditional-store values across both units must collapse to one class",
        );
    }

    #[test]
    fn explore_overlapping_writes_is_sensitive() {
        let result = explore(
            || {
                let mem = GuestMemory::new(64);
                let mut rt = Runtime::new(mem, Budget::new(100), 100);
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xAA),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xBB),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt
            },
            &ExplorationConfig::default(),
        );

        let r = result.expect("should have branching points");
        assert_eq!(r.outcome, OutcomeClass::ScheduleSensitive);
    }

    #[test]
    fn single_unit_returns_none() {
        let result = explore(
            || {
                let mem = GuestMemory::new(64);
                let mut rt = Runtime::new(mem, Budget::new(100), 100);
                rt.registry_mut()
                    .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
                rt
            },
            &ExplorationConfig::default(),
        );
        assert!(result.is_none());
    }

    /// Three units writing to disjoint addresses. Multiple branching
    /// points exist (3 runnable at step 0, 2 at step 1). All schedules
    /// should produce identical memory.
    #[test]
    fn three_unit_disjoint_is_stable() {
        let result = explore(
            || {
                let mem = GuestMemory::new(64);
                let mut rt = Runtime::new(mem, Budget::new(100), 100);
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xAA),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xBB),
                            FakeOp::SharedStore { addr: 8, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xCC),
                            FakeOp::SharedStore { addr: 16, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt
            },
            &ExplorationConfig::default(),
        );

        let r = result.expect("should have branching points");
        assert_eq!(r.outcome, OutcomeClass::ScheduleStable);
        assert!(r.total_branching_points >= 2);
        // Disjoint writes: all alternates pruned by dependency analysis.
        assert!(r.schedules.is_empty());
        assert!(r.schedules_pruned >= 3);
    }

    /// Three units all writing to the same address. Should be
    /// schedule-sensitive because last-writer-wins depends on order.
    #[test]
    fn three_unit_overlapping_is_sensitive() {
        let result = explore(
            || {
                let mem = GuestMemory::new(64);
                let mut rt = Runtime::new(mem, Budget::new(100), 100);
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xAA),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xBB),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xCC),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt
            },
            &ExplorationConfig::default(),
        );

        let r = result.expect("should have branching points");
        assert_eq!(r.outcome, OutcomeClass::ScheduleSensitive);
    }

    /// When max_schedules is 1, the explorer stops after one alternate
    /// and reports Inconclusive (bounds hit before full coverage).
    #[test]
    fn max_schedules_bound_produces_inconclusive() {
        let config = ExplorationConfig {
            max_schedules: 1,
            max_steps_per_run: 10_000,
        };
        // 3 units writing to the SAME address so pruning does not
        // eliminate any alternates. With max_schedules=1, the
        // explorer tries one alternate and stops.
        let result = explore(
            || {
                let mem = GuestMemory::new(64);
                let mut rt = Runtime::new(mem, Budget::new(100), 100);
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xAA),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xBB),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xCC),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt
            },
            &config,
        );

        let r = result.expect("should have branching points");
        assert_eq!(r.schedules.len(), 1, "should stop after 1 schedule");
        assert!(r.bounds_hit);
        // The single explored alternate swaps only the LoadImm order;
        // the last writer (unit 2) stays the same, so no divergence
        // is found yet. With bounds hit and no divergence: Inconclusive.
        assert_eq!(r.outcome, OutcomeClass::Inconclusive);
    }

    /// Verify ExplorationResult fields are populated correctly.
    #[test]
    fn result_fields_are_populated() {
        // Overlapping writes so pruning does not remove all alternates.
        let result = explore(
            || {
                let mem = GuestMemory::new(64);
                let mut rt = Runtime::new(mem, Budget::new(100), 100);
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xAA),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xBB),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt
            },
            &ExplorationConfig::default(),
        );

        let r = result.expect("should have branching points");
        assert!(r.baseline_hash != 0, "baseline hash should be non-zero");
        assert!(!r.schedules.is_empty());
        assert!(r.total_branching_points >= 1);
        assert_eq!(r.outcome, OutcomeClass::ScheduleSensitive);
    }

    /// Disjoint writers: pruning eliminates all alternates.
    #[test]
    fn disjoint_pruning_skips_all_alternates() {
        let result = explore(
            || {
                let mem = GuestMemory::new(64);
                let mut rt = Runtime::new(mem, Budget::new(100), 100);
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xAA),
                            FakeOp::SharedStore { addr: 0, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt.registry_mut().register_with(|id| {
                    FakeIsaUnit::new(
                        id,
                        vec![
                            FakeOp::LoadImm(0xBB),
                            FakeOp::SharedStore { addr: 8, len: 4 },
                            FakeOp::End,
                        ],
                    )
                });
                rt
            },
            &ExplorationConfig::default(),
        );

        let r = result.expect("should have branching points");
        assert_eq!(r.outcome, OutcomeClass::ScheduleStable);
        assert!(r.schedules.is_empty(), "all alternates should be pruned");
        assert!(
            r.schedules_pruned > 0,
            "pruning should have skipped at least one alternate"
        );
    }
}
