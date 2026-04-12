//! Oracle-aware exploration: run schedule exploration and extract
//! memory regions from each schedule for comparison against external
//! baselines (e.g., RPCS3).
//!
//! `explore_with_regions` is like `explore` but also captures named
//! memory regions after each run, allowing callers to compare each
//! schedule's output against oracle baselines.

use crate::classify::ExplorationResult;
use crate::config::ExplorationConfig;
use crate::observer::observe_decisions;
use crate::prescribed::PrescribedScheduler;
use crate::util::{build_overrides, classify_iteration, for_each_alternate, run_to_stall};
use cellgov_core::Runtime;
use cellgov_mem::{ByteRange, GuestAddr};

/// Specification for a memory region to extract after each run.
#[derive(Debug, Clone)]
pub struct MemoryRegionSpec {
    /// Human-readable region name.
    pub name: String,
    /// Guest address of the region start.
    pub addr: u64,
    /// Size in bytes.
    pub size: u64,
}

/// Captured memory region from a single run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedRegion {
    /// Region name (from the spec).
    pub name: String,
    /// Raw bytes extracted from committed memory.
    pub data: Vec<u8>,
}

/// Memory snapshot from one explored schedule.
#[derive(Debug, Clone)]
pub struct ScheduleSnapshot {
    /// Final committed-memory hash.
    pub memory_hash: u64,
    /// Extracted memory regions.
    pub regions: Vec<CapturedRegion>,
}

/// Result of an oracle-aware exploration run.
#[derive(Debug, Clone)]
pub struct OracleExplorationResult {
    /// The core exploration result (classification, schedules, pruning).
    pub exploration: ExplorationResult,
    /// Memory snapshot from the baseline (default-schedule) run.
    pub baseline: ScheduleSnapshot,
    /// Memory snapshots from each alternate schedule, parallel to
    /// `exploration.schedules`.
    pub alternates: Vec<ScheduleSnapshot>,
}

/// Run bounded schedule exploration and extract memory regions from
/// each schedule.
///
/// Like `explore`, but also captures the named memory regions after
/// each run for downstream oracle comparison. Returns `None` if the
/// baseline has no branching points.
pub fn explore_with_regions<F>(
    mut make_runtime: F,
    config: &ExplorationConfig,
    regions: &[MemoryRegionSpec],
) -> Option<OracleExplorationResult>
where
    F: FnMut() -> Runtime,
{
    let mut rt_baseline = make_runtime();
    let log = observe_decisions(&mut rt_baseline);
    let baseline_hash = rt_baseline.memory().content_hash();
    let baseline_regions = extract_regions(rt_baseline.memory(), regions);

    let total_branching_points = log.branching_count();
    if total_branching_points == 0 {
        return None;
    }

    let mut alternates = Vec::new();
    let iter = for_each_alternate(&log, config, baseline_hash, |step, alt| {
        let overrides = build_overrides(step, alt);
        let mut rt = make_runtime();
        rt.set_scheduler(PrescribedScheduler::new(overrides));
        run_to_stall(&mut rt, config.max_steps_per_run);
        let hash = rt.memory().content_hash();
        let captured = extract_regions(rt.memory(), regions);
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

/// Extract named memory regions from committed guest memory.
fn extract_regions(
    memory: &cellgov_mem::GuestMemory,
    specs: &[MemoryRegionSpec],
) -> Vec<CapturedRegion> {
    specs
        .iter()
        .map(|spec| {
            let data = ByteRange::new(GuestAddr::new(spec.addr), spec.size)
                .and_then(|range| memory.read(range))
                .map(|bytes| bytes.to_vec())
                .unwrap_or_else(|| vec![0u8; spec.size as usize]);
            CapturedRegion {
                name: spec.name.clone(),
                data,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::OutcomeClass;
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    #[test]
    fn explore_with_regions_captures_disjoint_writes() {
        let specs = vec![
            MemoryRegionSpec {
                name: "region_a".into(),
                addr: 0,
                size: 4,
            },
            MemoryRegionSpec {
                name: "region_b".into(),
                addr: 8,
                size: 4,
            },
        ];
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
                            FakeOp::SharedStore { addr: 8, len: 4 },
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
        // Baseline regions should have the written data.
        assert_eq!(r.baseline.regions.len(), 2);
        assert_eq!(r.baseline.regions[0].name, "region_a");
        assert_eq!(r.baseline.regions[0].data, vec![0xAA; 4]);
        assert_eq!(r.baseline.regions[1].name, "region_b");
        assert_eq!(r.baseline.regions[1].data, vec![0xBB; 4]);
    }

    #[test]
    fn overlapping_writes_regions_differ_across_schedules() {
        let specs = vec![MemoryRegionSpec {
            name: "shared".into(),
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
        assert_eq!(r.exploration.outcome, OutcomeClass::ScheduleSensitive);
        // At least one alternate should differ from baseline.
        let baseline_data = &r.baseline.regions[0].data;
        let any_different = r
            .alternates
            .iter()
            .any(|s| s.regions[0].data != *baseline_data);
        assert!(any_different, "at least one alternate should differ");
    }
}
