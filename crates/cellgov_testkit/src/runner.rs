//! The single canonical execution path. Build a runtime from a
//! [`ScenarioFixture`], drive it until it stalls or trips the deadlock
//! detector, capture the binary trace, snapshot final state hashes,
//! and return a structured [`ScenarioResult`].
//!
//! Tests build a fixture and hand it to [`run`]; the result is the
//! surface they assert against.
//!
//! ## What the runner does
//!
//! 1. Build a [`Runtime`] using the fixture's memory size, per-step
//!    budget, and max-steps cap.
//! 2. Run the fixture's one-shot registration callback against the
//!    runtime's `UnitRegistry`.
//! 3. Loop, calling `step()` and `commit_step()`, until either the
//!    scheduler returns `NoRunnableUnit` (clean stall: every unit
//!    finished or blocked) or the runtime returns `MaxStepsExceeded`
//!    (deadlock detector tripped).
//! 4. Snapshot the trace bytes, the final committed-memory hash, the
//!    final unit-status hash, the steps actually taken, and the
//!    terminal reason. Return them in [`ScenarioResult`].
//!
//! Commit failures from `commit_step()` are surfaced through the
//! existing fault-routing trace records (the runtime emits a
//! `CommitApplied { fault_discarded: true, ... }` record on validation
//! rejection); the runner does not abort on them. Stepping continues
//! until the scheduler is empty.

use crate::fixtures::ScenarioFixture;
use cellgov_core::{Runtime, StepError};
use cellgov_mem::GuestMemory;
use cellgov_trace::StateHash;

/// How a scenario run terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioOutcome {
    /// Every registered unit finished or blocked. The scheduler had
    /// nothing to run; the run is over. The expected terminal state
    /// for well-formed scenarios.
    Stalled,
    /// The runtime's max-steps cap fired before the scheduler emptied.
    /// The scenario looped, starved a unit, or otherwise failed to
    /// make progress; tests should treat this as a failure.
    MaxStepsExceeded,
}

/// Structured result of a scenario run.
///
/// Carries everything tests need to assert against: how the run ended,
/// how many steps it took, the captured binary trace, and the final
/// state hashes. Tests use the trace bytes via
/// [`cellgov_trace::TraceReader`] for golden-trace assertions and the
/// hashes for state-equivalence assertions.
#[derive(Debug, Clone)]
pub struct ScenarioResult {
    /// How the run terminated.
    pub outcome: ScenarioOutcome,
    /// Successful `Runtime::step` calls during the run. Does not count
    /// the final failing call that produced `outcome`.
    pub steps_taken: usize,
    /// Binary trace bytes captured from the runtime's writer. Decode
    /// with [`cellgov_trace::TraceReader`].
    pub trace_bytes: Vec<u8>,
    /// Hash of committed memory at the end of the run.
    pub final_memory_hash: StateHash,
    /// Hash of unit statuses at the end of the run.
    pub final_unit_status_hash: StateHash,
    /// Hash of the runtime's sync-registry contents at the end of the
    /// run. Aggregates mailboxes, signals, lv2 host state, and
    /// syscall responses into a single digest (see
    /// [`cellgov_core::Runtime::sync_state_hash`]).
    pub final_sync_hash: StateHash,
    /// Contents of the base-0 region at end of run. Auxiliary regions
    /// (stack, reserved) are not included. Used by the comparison
    /// harness to extract named memory regions.
    pub final_memory: Vec<u8>,
}

/// Drive a scenario fixture to completion via the canonical execution
/// path. See the module documentation for the contract.
pub fn run(fixture: ScenarioFixture) -> ScenarioResult {
    let mut memory = GuestMemory::new(fixture.memory_size);
    // Seed initial memory content (if any) before constructing the
    // runtime. Fixtures that need a pre-loaded image or trampoline
    // write it here.
    (fixture.seed_memory)(&mut memory);
    let mut rt = Runtime::new(memory, fixture.budget, fixture.max_steps);
    // Run the fixture's one-shot setup callback against the live
    // runtime so it can register units, mailboxes, signals, and any
    // other runtime-owned state in one place.
    (fixture.register)(&mut rt);

    let outcome = loop {
        match rt.step() {
            Ok(step) => {
                // Commit failures (validation rejection) are surfaced
                // by the runtime's commit trace record; the runner
                // does not abort. The next iteration handles whatever
                // state the failure left behind.
                let _ = rt.commit_step(&step.result, &step.effects);
            }
            Err(StepError::NoRunnableUnit) => break ScenarioOutcome::Stalled,
            Err(StepError::MaxStepsExceeded) => break ScenarioOutcome::MaxStepsExceeded,
            Err(StepError::TimeOverflow) => {
                // TimeOverflow is a runtime invariant violation rather
                // than a scenario outcome. Treat it as a stall in the
                // surfaced result so tests still see the trace and
                // hashes; tests that care will assert on the trace.
                break ScenarioOutcome::Stalled;
            }
        }
    };

    // Flush any DMA completions still in the queue. A scenario may
    // stall before enough guest time elapses for all enqueued DMAs to
    // fire on their scheduled tick; draining ensures the final memory
    // snapshot includes every committed transfer.
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
        // Trace should contain at least 5 UnitScheduled records.
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
            // A unit that never finishes -- forces the deadlock
            // detector to fire after exactly `max_steps` successful
            // steps.
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
        // Two fixtures built and run identically must produce
        // byte-identical traces and equal final hashes.
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
