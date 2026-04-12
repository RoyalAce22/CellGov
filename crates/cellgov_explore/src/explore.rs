//! Alternate-schedule replay.
//!
//! `explore_pair` runs the same fixture factory twice -- once with the
//! default round-robin scheduler, once with the first branching point's
//! alternate choice -- and returns both final memory hashes so the
//! caller can classify the workload as schedule-stable or
//! schedule-sensitive.

use crate::decision::DecisionLog;
use crate::observer::observe_decisions;
use crate::prescribed::PrescribedScheduler;
use crate::util::{build_overrides, run_to_stall};
use cellgov_core::Runtime;
use cellgov_event::UnitId;

/// Result of exploring two schedules for the same fixture.
#[derive(Debug, Clone)]
pub struct PairResult {
    /// Decision log from the default (first) run.
    pub log: DecisionLog,
    /// Final committed-memory hash from the default run.
    pub hash_a: u64,
    /// Final committed-memory hash from the alternate run.
    pub hash_b: u64,
    /// The branching point step index where the alternate diverged.
    pub branch_step: usize,
    /// The unit chosen in the alternate run at the branch point.
    pub alternate_choice: UnitId,
}

impl PairResult {
    /// Whether the two schedules produced identical memory.
    pub fn is_schedule_stable(&self) -> bool {
        self.hash_a == self.hash_b
    }
}

/// Run a fixture twice with different scheduling choices at the first
/// branching point. Returns `None` if the default run has no branching
/// points (nothing to explore).
///
/// `make_runtime` is called twice to produce two independent runtimes
/// from the same initial state. The first runs with the default
/// scheduler; the second gets a `PrescribedScheduler` that forces an
/// alternate unit at the first branching point.
pub fn explore_pair<F>(mut make_runtime: F) -> Option<PairResult>
where
    F: FnMut() -> Runtime,
{
    // Run 1: default schedule, record decisions.
    let mut rt_a = make_runtime();
    let log = observe_decisions(&mut rt_a);
    let hash_a = rt_a.memory().content_hash();

    // Find the first branching point.
    let branch = log.branching_points().next()?;
    let branch_step = branch.step;
    let default_choice = branch.chosen;

    // Pick the first alternative that isn't the default choice.
    let alternate_choice = branch
        .runnable
        .iter()
        .find(|&&uid| uid != default_choice)
        .copied()?;

    // Run 2: same fixture, prescribed scheduler.
    let overrides = build_overrides(branch_step, alternate_choice);
    let mut rt_b = make_runtime();
    rt_b.set_scheduler(PrescribedScheduler::new(overrides));
    run_to_stall(&mut rt_b, usize::MAX);
    let hash_b = rt_b.memory().content_hash();

    Some(PairResult {
        log,
        hash_a,
        hash_b,
        branch_step,
        alternate_choice,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    /// Two units writing to DISJOINT addresses. The final memory
    /// should be identical regardless of execution order.
    #[test]
    fn disjoint_writes_are_schedule_stable() {
        let result = explore_pair(|| {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            // Unit 0: write 0xAA to address 0..4
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
            // Unit 1: write 0xBB to address 8..12
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
        });

        let r = result.expect("should have a branching point");
        assert!(
            r.is_schedule_stable(),
            "disjoint writes should produce identical memory regardless of order"
        );
    }

    /// Two units writing to the SAME address. The final memory depends
    /// on who writes last, so alternate schedules should differ.
    #[test]
    fn overlapping_writes_are_schedule_sensitive() {
        let result = explore_pair(|| {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            // Unit 0: write 0xAA to address 0..4
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
            // Unit 1: write 0xBB to SAME address 0..4
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
        });

        let r = result.expect("should have a branching point");
        assert!(
            !r.is_schedule_stable(),
            "overlapping writes should produce different memory depending on order"
        );
    }

    /// Single unit: no branching points, explore_pair returns None.
    #[test]
    fn no_branching_returns_none() {
        let result = explore_pair(|| {
            let mem = GuestMemory::new(64);
            let mut rt = Runtime::new(mem, Budget::new(100), 100);
            rt.registry_mut()
                .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
            rt
        });
        assert!(result.is_none());
    }
}
