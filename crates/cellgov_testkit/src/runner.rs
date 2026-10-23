//! The canonical execution path. Build a runtime from a [`ScenarioFixture`],
//! step it until stall or max-steps, capture the trace and final hashes,
//! return a [`ScenarioResult`].
//!
//! Tests build a fixture and hand it to [`run`]; the result is the assertion
//! surface. The loop calls `step()` then `commit_step()` each iteration.
//! Stepping ends when the scheduler returns `NoRunnableUnit`/`AllBlocked`
//! (stall), on `MaxStepsExceeded`, or on the first refused commit.

use std::collections::BTreeMap;

use crate::fixtures::ScenarioFixture;
use cellgov_core::{AddressSpaceId, CommitError, MfcException, Runtime, StepError};
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, Region, RegionAccess};
use cellgov_trace::StateHash;

/// How a scenario run terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScenarioOutcome {
    /// Scheduler empty: every unit finished or blocked, and none faulted.
    /// Expected terminal state for well-formed scenarios.
    Stalled,
    /// Scheduler empty after a unit faulted. A faulted unit leaves the
    /// runnable set, so without this outcome the run would read as a
    /// stall; [`ScenarioResult::first_fault`] names the fault.
    Faulted,
    /// Max-steps cap fired before the scheduler emptied. Treated as failure.
    MaxStepsExceeded,
    /// The guest clock overflowed, an invariant violation. The trace and
    /// hashes stay available for inspection.
    TimeOverflow,
    /// The commit pipeline refused a step's batch. Its effects never
    /// reached guest state, so every later step would run on a state the
    /// schedule did not produce, and the run stops here.
    CommitRefused(CommitError),
}

/// Structured result of a scenario run; the assertion surface for tests.
#[derive(Debug, Clone)]
pub struct ScenarioResult {
    /// How the run terminated.
    pub outcome: ScenarioOutcome,
    /// Successful `Runtime::step` calls. Excludes the final failing call.
    /// It counts a step whose commit the pipeline refused.
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
    /// Every address space at end of run as independent copies; they
    /// share no backing with the runtime's memory, so holding a result
    /// never blocks a pooled memory's reset.
    pub final_spaces: BTreeMap<AddressSpaceId, GuestMemory>,
    /// The run's first host invariant break as one line, from
    /// `Lv2Observability::first_invariant_break_line`. The runtime ends
    /// here, so a driver that never sees it cannot report the break.
    pub first_invariant_break: Option<String>,
    /// The unit that faulted first, and its fault.
    pub first_fault: Option<(UnitId, FaultKind)>,
    /// The first MFC exception the run raised.
    ///
    /// A refused command suspends its SPU's command queue, and the run
    /// then stalls like any other. Only this field names the cause.
    pub mfc_exception: Option<MfcException>,
}

/// Copy a space's regions into fresh backing, keeping each region's
/// access mode so a read through the copy resolves as it does in the
/// source.
fn deep_copy(mem: &GuestMemory) -> GuestMemory {
    let regions = mem
        .regions()
        .map(|r| {
            Region::with_access(
                r.base(),
                r.bytes().len(),
                r.label(),
                r.page_size(),
                r.access(),
            )
        })
        .collect();
    let mut copy =
        GuestMemory::from_regions(regions).expect("copying non-overlapping regions cannot overlap");
    for r in mem.regions() {
        if r.access() != RegionAccess::ReadWrite {
            // `apply_commit` refuses reserved regions, so their backing
            // never left the zeros construction gave it; the copy's
            // fresh zeros already match.
            debug_assert!(
                r.bytes().iter().all(|&b| b == 0),
                "reserved region {} holds non-zero bytes",
                r.label()
            );
            continue;
        }
        let range = ByteRange::new(GuestAddr::new(r.base()), r.size())
            .expect("a mapped region's range fits the address space");
        copy.apply_commit(range, r.bytes())
            .expect("the copy maps every range the source does");
    }
    copy
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

    let mut first_fault = None;
    let outcome = loop {
        match rt.step() {
            Ok(mut step) => {
                if first_fault.is_none() {
                    first_fault = step.result.fault.map(|fault| (step.unit, fault));
                }
                if let Err(e) = rt.commit_step(&mut step) {
                    break ScenarioOutcome::CommitRefused(e);
                }
            }
            Err(StepError::NoRunnableUnit) | Err(StepError::AllBlocked) => {
                break if first_fault.is_some() {
                    ScenarioOutcome::Faulted
                } else {
                    ScenarioOutcome::Stalled
                };
            }
            Err(StepError::MaxStepsExceeded) => break ScenarioOutcome::MaxStepsExceeded,
            Err(StepError::TimeOverflow) => break ScenarioOutcome::TimeOverflow,
            Err(StepError::SchedulerNotReinstalled) => {
                // The runner builds a fresh runtime and never calls
                // restore_into, so this arm is unreachable.
                unreachable!("testkit runner does not call Runtime::restore_into");
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
        final_memory_hash: StateHash::new(rt.committed_memory_hash()),
        final_unit_status_hash: StateHash::new(rt.registry().status_hash()),
        final_sync_hash: StateHash::new(rt.sync_state_hash()),
        final_memory: rt.memory().as_bytes().to_vec(),
        final_spaces: rt
            .address_spaces()
            .map(|(id, mem)| (id, deep_copy(mem)))
            .collect(),
        first_invariant_break: rt.lv2_host().observability().first_invariant_break_line(),
        first_fault,
        mfc_exception: rt.take_mfc_exception(),
    };
    let mem = rt.into_memory();
    (result, mem)
}

#[cfg(test)]
#[path = "tests/runner_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/deep_copy_tests.rs"]
mod deep_copy_tests;

#[cfg(test)]
#[path = "tests/runner_commit_refusal_tests.rs"]
mod commit_refusal_tests;
