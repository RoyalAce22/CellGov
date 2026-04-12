//! Decision observer -- runs a scenario and records every scheduling
//! decision the runtime makes, including the full runnable set at
//! each step.

use crate::decision::{DecisionLog, DecisionPoint};
use crate::dependency::StepFootprint;
use cellgov_core::Runtime;

/// Run the runtime to completion (stall or max-steps) and record
/// every scheduling decision into a `DecisionLog`.
///
/// Returns the log and the number of steps taken. The runtime is
/// consumed; callers who need the final state should inspect it
/// before calling this, or use the log to identify branching points
/// for replay.
pub fn observe_decisions(rt: &mut Runtime) -> DecisionLog {
    let mut log = DecisionLog::new();
    loop {
        let runnable: Vec<_> = rt.registry().runnable_ids().collect();
        if runnable.is_empty() {
            break;
        }
        let step_idx = rt.steps_taken();
        match rt.step() {
            Ok(step) => {
                let footprint = StepFootprint::from_effects(&step.result.emitted_effects);
                let _ = rt.commit_step(&step.result);
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
    log
}

#[cfg(test)]
mod tests {
    use super::*;
    use cellgov_core::Runtime;
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    /// Two independent units that each run one step and finish.
    /// The first step should have both runnable (a branching point);
    /// the second step should have only one (not branching).
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

    /// A single unit that finishes in 1 step. No branching points.
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
