//! Runs the baseline schedule and records every scheduling decision
//! with the full runnable set at each step.

use crate::decision::{DecisionLog, DecisionPoint};
use crate::dependency::StepFootprint;
use cellgov_core::{Runtime, RuntimeSnapshot};
use std::collections::BTreeMap;

/// Drive `rt` to stall and return the recorded [`DecisionLog`].
///
/// The runtime is advanced in place; callers who need the final state
/// should inspect `rt` after the call.
pub fn observe_decisions(rt: &mut Runtime) -> DecisionLog {
    let (log, _) = observe_decisions_with_snapshots(rt, false);
    log
}

/// Like [`observe_decisions`], but with `capture=true` also records
/// a [`RuntimeSnapshot`] keyed by step index at every branching
/// point (>=2 runnable units). Skipping non-branching steps bounds
/// peak memory to `branching_points * sizeof(snapshot)`.
pub fn observe_decisions_with_snapshots(
    rt: &mut Runtime,
    capture: bool,
) -> (DecisionLog, BTreeMap<usize, RuntimeSnapshot>) {
    let mut log = DecisionLog::new();
    let mut snapshots: BTreeMap<usize, RuntimeSnapshot> = BTreeMap::new();
    loop {
        let runnable: Vec<_> = rt.registry().runnable_ids().collect();
        if runnable.is_empty() {
            break;
        }
        let step_idx = rt.steps_taken();
        if capture && runnable.len() >= 2 {
            // Snapshot must precede the step so alternates replay the
            // branching-point step itself with a different choice.
            snapshots.insert(step_idx, rt.snapshot());
        }
        match rt.step() {
            Ok(step) => {
                let footprint = StepFootprint::from_effects(&step.effects);
                let _ = rt.commit_step(&step.result, &step.effects);
                log.push(DecisionPoint {
                    step: step_idx,
                    runnable,
                    chosen: step.unit,
                    footprint,
                });
            }
            Err(_) => break,
        }
    }
    (log, snapshots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_core::Runtime;
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    #[test]
    fn two_units_produces_branching_point() {
        let mem = GuestMemory::new(64);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);

        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));

        let log = observe_decisions(&mut rt);
        assert!(
            log.len() >= 2,
            "expected at least 2 steps, got {}",
            log.len()
        );
        assert!(
            log.points()[0].is_branching(),
            "first step should be a branching point (2 runnable)"
        );
        assert!(
            log.branching_count() >= 1,
            "expected at least 1 branching point, got {}",
            log.branching_count()
        );
    }

    #[test]
    fn single_unit_no_branching() {
        let mem = GuestMemory::new(64);
        let mut rt = Runtime::new(mem, Budget::new(100), 100);

        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));

        let log = observe_decisions(&mut rt);
        assert_eq!(log.len(), 1);
        assert_eq!(log.branching_count(), 0);
    }
}
