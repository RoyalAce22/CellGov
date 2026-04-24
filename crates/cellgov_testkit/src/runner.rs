//! The canonical execution path. Build a runtime from a [`ScenarioFixture`],
//! step it until stall or max-steps, capture the trace and final hashes,
//! return a [`ScenarioResult`].
//!
//! Tests build a fixture and hand it to [`run`]; the result is the assertion
//! surface. The loop calls `step()` then `commit_step()` each iteration;
//! commit failures surface as `fault_discarded` trace records and do not
//! abort the run. Stepping ends when the scheduler returns
//! `NoRunnableUnit`/`AllBlocked` (stall) or `MaxStepsExceeded`.

use crate::fixtures::ScenarioFixture;
use cellgov_core::{Runtime, StepError};
use cellgov_mem::GuestMemory;
use cellgov_trace::StateHash;

/// How a scenario run terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioOutcome {
    /// Scheduler empty: every unit finished or blocked. Expected terminal
    /// state for well-formed scenarios.
    Stalled,
    /// Max-steps cap fired before the scheduler emptied. Treated as failure.
    MaxStepsExceeded,
}

/// Structured result of a scenario run; the assertion surface for tests.
#[derive(Debug, Clone)]
pub struct ScenarioResult {
    /// How the run terminated.
    pub outcome: ScenarioOutcome,
    /// Successful `Runtime::step` calls. Excludes the final failing call.
    pub steps_taken: usize,
    /// Binary trace bytes. Decode with [`cellgov_trace::TraceReader`].
    pub trace_bytes: Vec<u8>,
    /// Committed-memory hash at end of run.
    pub final_memory_hash: StateHash,
    /// Unit-status hash at end of run.
    pub final_unit_status_hash: StateHash,
    /// Combined sync-registry hash; see [`cellgov_core::Runtime::sync_state_hash`].
    pub final_sync_hash: StateHash,
    /// Base-0 region bytes at end of run. Auxiliary regions not included.
    pub final_memory: Vec<u8>,
}

/// Drive a scenario fixture to completion via the canonical path.
pub fn run(fixture: ScenarioFixture) -> ScenarioResult {
    let mut memory = GuestMemory::new(fixture.memory_size);
    (fixture.seed_memory)(&mut memory);
    let mut rt = Runtime::new(memory, fixture.budget, fixture.max_steps);
    (fixture.register)(&mut rt);

    let outcome = loop {
        match rt.step() {
            Ok(step) => {
                let _ = rt.commit_step(&step.result, &step.effects);
            }
            Err(StepError::NoRunnableUnit) | Err(StepError::AllBlocked) => {
                break ScenarioOutcome::Stalled;
            }
            Err(StepError::MaxStepsExceeded) => break ScenarioOutcome::MaxStepsExceeded,
            Err(StepError::TimeOverflow) => {
                // Invariant violation; surface as stall so the trace and
                // hashes are still available for inspection.
                break ScenarioOutcome::Stalled;
            }
        }
    };

    // DMA completions still in-flight at stall must land before the final
    // memory snapshot is taken.
    rt.drain_pending_dma();

    ScenarioResult {
        outcome,
        steps_taken: rt.steps_taken(),
        trace_bytes: rt.trace().bytes().to_vec(),
        final_memory_hash: StateHash::new(rt.memory().content_hash()),
        final_unit_status_hash: StateHash::new(rt.registry().status_hash()),
        final_sync_hash: StateHash::new(rt.sync_state_hash()),
        final_memory: rt.memory().as_bytes().to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::CountingUnit;
    use cellgov_core::Runtime;
    use cellgov_time::Budget;
    use cellgov_trace::{TraceReader, TraceRecord};

    #[test]
    fn empty_fixture_stalls_immediately_with_no_steps() {
        let result = run(ScenarioFixture::empty());
        assert_eq!(result.outcome, ScenarioOutcome::Stalled);
        assert_eq!(result.steps_taken, 0);
        assert!(result.trace_bytes.is_empty());
    }

    #[test]
    fn single_unit_runs_to_completion_then_stalls() {
        let result = run(ScenarioFixture::builder()
            .memory_size(16)
            .budget(Budget::new(1))
            .max_steps(100)
            .register(|rt: &mut Runtime| {
                let r = rt.registry_mut();
                r.register_with(|id| CountingUnit::new(id, 5));
            })
            .build());
        assert_eq!(result.outcome, ScenarioOutcome::Stalled);
        assert_eq!(result.steps_taken, 5);
        let scheduled_count = TraceReader::new(&result.trace_bytes)
            .map(|r| r.expect("decode"))
            .filter(|r| matches!(r, TraceRecord::UnitScheduled { .. }))
            .count();
        assert_eq!(scheduled_count, 5);
    }

    #[test]
    fn max_steps_cap_surfaces_as_max_steps_exceeded() {
        let result = run(ScenarioFixture::builder()
            .memory_size(16)
            .budget(Budget::new(1))
            .max_steps(3)
            .register(|rt: &mut Runtime| {
                let r = rt.registry_mut();
                r.register_with(|id| CountingUnit::new(id, u64::MAX));
            })
            .build());
        assert_eq!(result.outcome, ScenarioOutcome::MaxStepsExceeded);
        assert_eq!(result.steps_taken, 3);
    }

    #[test]
    fn two_runs_of_the_same_fixture_are_byte_identical() {
        fn build_and_run() -> ScenarioResult {
            run(ScenarioFixture::builder()
                .memory_size(16)
                .budget(Budget::new(2))
                .max_steps(100)
                .register(|rt: &mut Runtime| {
                    let r = rt.registry_mut();
                    r.register_with(|id| CountingUnit::new(id, 4));
                    r.register_with(|id| CountingUnit::new(id, 6));
                })
                .build())
        }
        let a = build_and_run();
        let b = build_and_run();
        assert_eq!(a.outcome, b.outcome);
        assert_eq!(a.steps_taken, b.steps_taken);
        assert_eq!(a.trace_bytes, b.trace_bytes);
        assert_eq!(a.final_memory_hash, b.final_memory_hash);
        assert_eq!(a.final_unit_status_hash, b.final_unit_status_hash);
    }
}
