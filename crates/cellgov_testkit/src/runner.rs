//! The canonical execution path. Build a runtime from a [`ScenarioFixture`],
//! step it until stall or max-steps, capture the trace and final hashes,
//! return a [`ScenarioResult`].
//!
//! Tests build a fixture and hand it to [`run`]; the result is the assertion
//! surface. The loop calls `step()` then `commit_step()` each iteration;
//! commit failures surface as `fault_discarded` trace records and do not
//! abort the run. Stepping ends when the scheduler returns
//! `NoRunnableUnit`/`AllBlocked` (stall) or `MaxStepsExceeded`.

use std::collections::BTreeMap;

use crate::fixtures::ScenarioFixture;
use cellgov_core::{AddressSpaceId, Runtime, StepError};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, Region, RegionAccess};
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
    /// Every address space at end of run as independent copies; they
    /// share no backing with the runtime's memory, so holding a result
    /// never blocks a pooled memory's reset.
    pub final_spaces: BTreeMap<AddressSpaceId, GuestMemory>,
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
    };
    let mem = rt.into_memory();
    (result, mem)
}

#[cfg(test)]
#[path = "tests/runner_tests.rs"]
mod tests;

#[cfg(test)]
mod deep_copy_tests {
    use super::*;
    use crate::world::WritingUnit;
    use cellgov_mem::PageSize;
    use cellgov_time::Budget;

    const CHILD: AddressSpaceId = AddressSpaceId::new(1);

    fn two_space_fixture() -> ScenarioFixture {
        ScenarioFixture::builder()
            .memory_size(16)
            .budget(Budget::new(1))
            .max_steps(10)
            .register(|rt: &mut Runtime| {
                rt.create_address_space(CHILD).unwrap();
                let mem = rt.space_memory_mut(CHILD).unwrap();
                *mem = GuestMemory::from_regions(vec![
                    Region::new(0, 16, "child_main", PageSize::Page4K),
                    Region::with_access(
                        0x1000,
                        16,
                        "zero_readable",
                        PageSize::Page4K,
                        RegionAccess::ReservedZeroReadable,
                    ),
                    Region::with_access(
                        0x2000,
                        16,
                        "strict",
                        PageSize::Page4K,
                        RegionAccess::ReservedStrict,
                    ),
                ])
                .unwrap();
                let seed = ByteRange::new(GuestAddr::new(0), 4).unwrap();
                mem.apply_commit(seed, &[1, 2, 3, 4]).unwrap();
                rt.registry_mut()
                    .register_with(|id| WritingUnit::at_zero(id, 2));
            })
            .build()
    }

    fn head(mem: &GuestMemory, addr: u64) -> Option<Vec<u8>> {
        mem.read(ByteRange::new(GuestAddr::new(addr), 4).unwrap())
            .map(<[u8]>::to_vec)
    }

    #[test]
    fn the_boot_copy_holds_the_last_committed_write() {
        let result = run(two_space_fixture());
        let boot = &result.final_spaces[&AddressSpaceId::BOOT];
        assert_eq!(head(boot, 0), Some(vec![2, 2, 2, 2]));
        assert_eq!(&result.final_memory[..4], &[2, 2, 2, 2]);
    }

    #[test]
    fn a_child_copy_keeps_every_region_access_mode() {
        let result = run(two_space_fixture());
        let child = &result.final_spaces[&CHILD];
        let modes: Vec<(&str, RegionAccess)> =
            child.regions().map(|r| (r.label(), r.access())).collect();
        assert_eq!(
            modes,
            vec![
                ("child_main", RegionAccess::ReadWrite),
                ("zero_readable", RegionAccess::ReservedZeroReadable),
                ("strict", RegionAccess::ReservedStrict),
            ]
        );
        assert_eq!(head(child, 0), Some(vec![1, 2, 3, 4]));
        assert_eq!(head(child, 0x1000), Some(vec![0; 4]));
        assert_eq!(head(child, 0x2000), None);
    }

    #[test]
    fn a_single_space_copy_hashes_like_the_runtime_memory() {
        let result = run(ScenarioFixture::builder()
            .memory_size(16)
            .budget(Budget::new(1))
            .max_steps(10)
            .register(|rt: &mut Runtime| {
                rt.registry_mut()
                    .register_with(|id| WritingUnit::at_zero(id, 3));
            })
            .build());
        assert_eq!(result.final_spaces.len(), 1);
        let boot = &result.final_spaces[&AddressSpaceId::BOOT];
        assert_eq!(boot.content_hash(), result.final_memory_hash.raw());
    }

    #[test]
    fn a_pooled_rerun_resets_while_an_earlier_result_is_still_held() {
        let mut pool = MemoryPool::new();
        let first = run_pooled(two_space_fixture(), &mut pool);
        let second = run_pooled(two_space_fixture(), &mut pool);
        let boot = &first.final_spaces[&AddressSpaceId::BOOT];
        assert_eq!(head(boot, 0), Some(vec![2, 2, 2, 2]));
        assert_eq!(first.final_memory_hash, second.final_memory_hash);
    }
}
