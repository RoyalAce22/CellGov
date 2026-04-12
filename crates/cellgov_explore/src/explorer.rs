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
