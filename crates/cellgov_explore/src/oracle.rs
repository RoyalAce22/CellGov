//! Exploration wrapper that also captures named memory regions from
//! each run for comparison against external baselines.

use crate::classify::ExplorationResult;
use crate::config::ExplorationConfig;
use crate::observer::observe_decisions_with_snapshots;
use crate::prescribed::PrescribedScheduler;
use crate::util::{classify_iteration, for_each_alternate, run_to_stall};
use cellgov_core::Runtime;
use cellgov_mem::{ByteRange, GuestAddr};

/// One named memory region to capture after each run.
#[derive(Debug, Clone)]
pub struct MemoryRegionSpec {
    /// Human-readable region name.
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
}

/// Bytes captured from one region of one run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedRegion {
    /// Region name (from the spec).
    pub name: String,
    /// Raw bytes from committed memory; empty when `resolved` is false.
    pub data: Vec<u8>,
    /// False when the spec's range could not be read from this run's
    /// committed memory (unmapped address or overflowing range).
    ///
    /// `data` is left empty in that case rather than zero-filled: a
    /// fabricated all-zero capture is indistinguishable from a real
    /// zeroed region and would compare equal against an all-zero
    /// oracle baseline.
    pub resolved: bool,
}

/// Memory snapshot from one explored schedule.
#[derive(Debug, Clone)]
pub struct ScheduleSnapshot {
    /// Final committed-memory hash.
    pub memory_hash: u64,
    /// Captured regions, in spec order.
    pub regions: Vec<CapturedRegion>,
}

/// Result of an oracle-aware exploration run.
///
/// `alternates` is parallel to `exploration.schedules`.
#[derive(Debug, Clone)]
pub struct OracleExplorationResult {
    /// Core exploration verdict and per-alternate records.
    pub exploration: ExplorationResult,
    /// Snapshot from the baseline run.
    pub baseline: ScheduleSnapshot,
    /// Snapshot from each non-pruned alternate, in exploration order.
    pub alternates: Vec<ScheduleSnapshot>,
}

/// Like [`crate::explore()`] but also captures named regions from every
/// run.
///
/// Returns `None` if the baseline has no branching points.
pub fn explore_with_regions<F>(
    mut make_runtime: F,
    config: &ExplorationConfig,
    regions: &[MemoryRegionSpec],
) -> Option<OracleExplorationResult>
where
    F: FnMut() -> Runtime,
{
    let mut rt_baseline = make_runtime();
    let (log, snapshots) = observe_decisions_with_snapshots(&mut rt_baseline, true);
    let baseline_hash = rt_baseline.committed_memory_hash();
    let baseline_regions = extract_regions(rt_baseline.memory(), regions);

    let total_branching_points = log.branching_count();
    if total_branching_points == 0 {
        return None;
    }

    let mut alternates = Vec::new();
    let iter = for_each_alternate(&log, config, baseline_hash, |step, alt| {
        let snap = snapshots
            .get(&step)
            .expect("observer must snapshot every branching point");
        rt_baseline.restore_into(snap);
        rt_baseline.set_scheduler(PrescribedScheduler::single_choice(alt));
        run_to_stall(&mut rt_baseline, config.max_steps_per_run);
        let hash = rt_baseline.committed_memory_hash();
        let captured = extract_regions(rt_baseline.memory(), regions);
        alternates.push(ScheduleSnapshot {
            memory_hash: hash,
            regions: captured,
        });
        hash
    });

    let exploration = classify_iteration(iter, baseline_hash, total_branching_points);
    Some(OracleExplorationResult {
        exploration,
        baseline: ScheduleSnapshot {
            memory_hash: baseline_hash,
            regions: baseline_regions,
        },
        alternates,
    })
}

fn extract_regions(
    memory: &cellgov_mem::GuestMemory,
    specs: &[MemoryRegionSpec],
) -> Vec<CapturedRegion> {
    specs
        .iter()
        .map(|spec| {
            match ByteRange::new(GuestAddr::new(spec.addr), spec.size)
                .and_then(|range| memory.read(range))
            {
                Some(bytes) => CapturedRegion {
                    name: spec.name.clone(),
                    data: bytes.to_vec(),
                    resolved: true,
                },
                None => CapturedRegion {
                    name: spec.name.clone(),
                    data: Vec::new(),
                    resolved: false,
                },
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/oracle_tests.rs"]
mod tests;

#[cfg(test)]
mod capture_tests {
    use super::*;
    use crate::config::ExplorationConfig;
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    #[test]
    fn an_unmapped_region_spec_is_captured_unresolved_not_zero_filled() {
        // The 64-byte fixture memory ends well before 0x1_0000, so the
        // spec's range cannot be read from any run's committed memory.
        let specs = vec![MemoryRegionSpec {
            name: "outside".into(),
            addr: 0x1_0000,
            size: 4,
        }];
        let result = explore_with_regions(
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
            &specs,
        );

        let r = result.expect("should have branching points");
        assert!(!r.baseline.regions[0].resolved);
        assert!(r.baseline.regions[0].data.is_empty());
        assert!(
            !r.alternates.is_empty(),
            "overlapping writers must produce at least one explored alternate"
        );
        for alt in &r.alternates {
            assert!(!alt.regions[0].resolved);
            assert!(alt.regions[0].data.is_empty());
        }
    }

    #[test]
    fn a_mapped_region_spec_is_captured_resolved() {
        let specs = vec![MemoryRegionSpec {
            name: "inside".into(),
            addr: 0,
            size: 4,
        }];
        let result = explore_with_regions(
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
            &specs,
        );

        let r = result.expect("should have branching points");
        assert!(r.baseline.regions[0].resolved);
        assert_eq!(r.baseline.regions[0].data.len(), 4);
    }
}
