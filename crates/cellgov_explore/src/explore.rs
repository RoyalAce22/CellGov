//! Two-schedule exploration: default run vs a single alternate at the
//! first branching point.

use crate::decision::DecisionLog;
use crate::observer::observe_decisions;
use crate::prescribed::PrescribedScheduler;
use crate::util::{build_overrides, run_to_stall};
use cellgov_core::Runtime;
use cellgov_event::UnitId;

/// Outcome of exploring two schedules for the same fixture.
#[derive(Debug, Clone)]
pub struct PairResult {
    /// Decision log from the default (first) run.
    pub log: DecisionLog,
    /// Committed-memory hash from the default run.
    pub hash_a: u64,
    /// Committed-memory hash from the alternate run.
    pub hash_b: u64,
    /// Step index of the branching point the alternate diverges at.
    pub branch_step: usize,
    /// Unit chosen at the branch point in the alternate run.
    pub alternate_choice: UnitId,
}

impl PairResult {
    /// True when the two schedules committed identical memory.
    pub fn is_schedule_stable(&self) -> bool {
        self.hash_a == self.hash_b
    }
}

/// Run the fixture twice with different choices at the first branching
/// point.
///
/// Returns `None` if the default run has no branching points.
/// `make_runtime` is called twice and must produce independent runtimes
/// from identical initial state.
pub fn explore_pair<F>(mut make_runtime: F) -> Option<PairResult>
where
    F: FnMut() -> Runtime,
{
    let mut rt_a = make_runtime();
    let log = observe_decisions(&mut rt_a);
    let hash_a = rt_a.memory().content_hash();

    let branch = log.branching_points().next()?;
    let branch_step = branch.step;
    let default_choice = branch.chosen;

    let alternate_choice = branch
        .runnable
        .iter()
        .find(|&&uid| uid != default_choice)
        .copied()?;

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

    #[test]
    fn disjoint_writes_are_schedule_stable() {
        let result = explore_pair(|| {
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
        });

        let r = result.expect("should have a branching point");
        assert!(
            r.is_schedule_stable(),
            "disjoint writes should produce identical memory regardless of order"
        );
    }

    #[test]
    fn overlapping_writes_are_schedule_sensitive() {
        let result = explore_pair(|| {
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
        });

        let r = result.expect("should have a branching point");
        assert!(
            !r.is_schedule_stable(),
            "overlapping writes should produce different memory depending on order"
        );
    }

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
