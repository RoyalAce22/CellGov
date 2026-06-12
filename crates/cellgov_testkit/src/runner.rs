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
    let memory = GuestMemory::new(fixture.memory_size);
    let (result, _mem) = run_internal(fixture, memory);
    result
}

/// One-`GuestMemory`-per-size cache for tests that run many
/// scenarios at the same `memory_size`. Not thread-safe;
/// instantiate per thread via `thread_local!`.
#[derive(Debug, Default)]
pub struct MemoryPool {
    cached: Option<GuestMemory>,
}

impl MemoryPool {
    /// Empty pool; first call to a pooled runner allocates the
    /// backing `GuestMemory`.
    pub fn new() -> Self {
        Self { cached: None }
    }
}

/// Drive a scenario fixture using a pooled [`GuestMemory`]. The
/// cached memory is reset via [`GuestMemory::reset_for_reuse`]
/// (`O(touched pages)`); a size mismatch discards the cache and
/// allocates fresh.
///
/// # Panics
///
/// Panics if the pooled memory's `Arc<Vec<u8>>` backing is held by
/// an outstanding snapshot at reset time.
pub fn run_pooled(fixture: ScenarioFixture, pool: &mut MemoryPool) -> ScenarioResult {
    let memory = match pool.cached.take() {
        Some(mut mem) if mem.size() == fixture.memory_size as u64 => {
            mem.reset_for_reuse();
            mem
        }
        _ => GuestMemory::new(fixture.memory_size),
    };
    let (result, mem) = run_internal(fixture, memory);
    pool.cached = Some(mem);
    result
}

fn run_internal(fixture: ScenarioFixture, memory: GuestMemory) -> (ScenarioResult, GuestMemory) {
    let mut memory = memory;
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
                // Invariant violation; surface as stall so the trace
                // and hashes are still available for inspection.
                break ScenarioOutcome::Stalled;
            }
            Err(StepError::SchedulerNotReinstalled) => {
                // Programming error, not a guest/terminal condition.
                // The testkit runner builds a fresh runtime and never
                // calls restore_into, so this arm is provably
                // unreachable. Folding it into ScenarioOutcome::Stalled
                // would silently truncate the run with no diagnostic
                // naming the wiring mistake; panic instead so the
                // misuse is loud at the site that introduced it.
                unreachable!(
                    "testkit runner does not call Runtime::restore_into; \
                     reaching this arm means a new caller added a \
                     restore path without rethinking the dispatch."
                );
            }
        }
    };

    // DMA completions still in-flight at stall must land before the final
    // memory snapshot is taken.
    rt.drain_pending_dma();

    let result = ScenarioResult {
        outcome,
        steps_taken: rt.steps_taken(),
        trace_bytes: rt.trace().bytes().to_vec(),
        final_memory_hash: StateHash::new(rt.memory().content_hash()),
        final_unit_status_hash: StateHash::new(rt.registry().status_hash()),
        final_sync_hash: StateHash::new(rt.sync_state_hash()),
        final_memory: rt.memory().as_bytes().to_vec(),
    };
    let mem = rt.into_memory();
    (result, mem)
}

#[cfg(test)]
#[path = "tests/runner_tests.rs"]
mod tests;
