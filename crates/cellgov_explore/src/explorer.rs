//! Main bounded enumerator: baseline run then one replay per
//! non-pruned alternate branching-point choice.

use crate::classify::ExplorationResult;
use crate::config::ExplorationConfig;
use crate::observer::observe_decisions;
use crate::prescribed::PrescribedScheduler;
use crate::util::{build_overrides, classify_iteration, for_each_alternate, run_to_stall};
use cellgov_core::Runtime;

/// Run bounded schedule exploration on a workload.
///
/// Returns `None` if the baseline run has no branching points.
///
/// `make_runtime` is called once for the baseline and once per
/// non-pruned alternate; each call must produce an independent runtime
/// from identical initial state. Exploration stops after
/// `config.max_schedules` alternates or when every alternate has been
/// replayed, and each replay stops after `config.max_steps_per_run`
/// steps.
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

        let r1 = explore(make, &ExplorationConfig::default())
            .expect("contention workload must have branching points");
        let r2 = explore(make, &ExplorationConfig::default())
            .expect("contention workload must have branching points");

        assert_eq!(
            r1.outcome, r2.outcome,
            "exploration classification must be stable across runs",
        );
        assert!(matches!(
            r1.outcome,
            OutcomeClass::ScheduleStable | OutcomeClass::ScheduleSensitive
        ));
    }

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
        assert!(r.schedules.is_empty());
        assert!(r.schedules_pruned >= 3);
    }

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

    #[test]
    fn max_schedules_bound_produces_inconclusive() {
        let config = ExplorationConfig {
            max_schedules: 1,
            max_steps_per_run: 10_000,
        };
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
        // The one explored alternate swaps LoadImm order but not the
        // last writer, so no divergence is visible; bounds-hit without
        // divergence classifies as Inconclusive.
        assert_eq!(r.outcome, OutcomeClass::Inconclusive);
    }

    #[test]
    fn result_fields_are_populated() {
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
