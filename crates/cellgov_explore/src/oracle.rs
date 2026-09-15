//! Exploration wrapper that also captures named memory regions from
//! each run for comparison against external baselines.

use crate::classify::{BaselineRun, ExplorationResult};
use crate::config::ExplorationConfig;
use crate::observer::observe_decisions_with_snapshots;
use crate::prescribed::PrescribedScheduler;
use crate::util::{classify_iteration, for_each_alternate, run_to_stall};
use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_mem::{ByteRange, GuestAddr};

/// One named memory region to capture after each run.
#[derive(Debug, Clone)]
pub struct MemoryRegionSpec {
    /// Human-readable region name.
    pub name: String,
    /// Address space the region lives in; equal numeric addresses in
    /// different spaces name different memory.
    pub space: AddressSpaceId,
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
    /// committed memory: a space the run never created, an unmapped
    /// address, or an overflowing range.
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
/// Returns `None` if the baseline has no branching points. A baseline
/// that stopped short withdraws every divergence claim, exactly as in
/// [`crate::explore()`].
pub fn explore_with_regions<F>(
    mut make_runtime: F,
    config: &ExplorationConfig,
    regions: &[MemoryRegionSpec],
) -> Option<OracleExplorationResult>
where
    F: FnMut() -> Runtime,
{
    let mut rt_baseline = make_runtime();
    let (log, snapshots, baseline_stop) = observe_decisions_with_snapshots(&mut rt_baseline, true);
    let baseline = BaselineRun {
        hash: rt_baseline.committed_memory_hash(),
        steps: log.len(),
        stop: baseline_stop,
    };
    let baseline_hash = baseline.hash;
    let baseline_regions = extract_regions(&rt_baseline, regions);

    let total_branching_points = log.branching_count();
    if total_branching_points == 0 {
        return None;
    }

    // Read before the first replay: `Runtime::restore_into` overwrites
    // the whole LV2 host from the snapshot, so the baseline's record is
    // gone the moment an alternate restores over it.
    let mut first_invariant_break = rt_baseline
        .lv2_host()
        .observability()
        .first_invariant_break_line();

    let mut alternates = Vec::new();
    let mut iter = for_each_alternate(&log, config, baseline_hash, |step, alt| {
        let snap = snapshots
            .get(&step)
            .expect("observer must snapshot every branching point");
        rt_baseline.restore_into(snap);
        rt_baseline.set_scheduler(PrescribedScheduler::single_choice(alt));
        let stop = run_to_stall(&mut rt_baseline, config.max_steps_per_run);
        // The next replay restores over this one's record, so a break
        // only this replay found is readable only here.
        if first_invariant_break.is_none() {
            first_invariant_break = rt_baseline
                .lv2_host()
                .observability()
                .first_invariant_break_line();
        }
        let hash = rt_baseline.committed_memory_hash();
        let captured = extract_regions(&rt_baseline, regions);
        alternates.push(ScheduleSnapshot {
            memory_hash: hash,
            regions: captured,
        });
        (hash, stop)
    });

    if baseline.stop.is_truncated() {
        iter.mark_baseline_truncated();
    }

    let exploration = classify_iteration(
        iter,
        baseline,
        total_branching_points,
        first_invariant_break,
    );
    Some(OracleExplorationResult {
        exploration,
        baseline: ScheduleSnapshot {
            memory_hash: baseline_hash,
            regions: baseline_regions,
        },
        alternates,
    })
}

fn extract_regions(rt: &Runtime, specs: &[MemoryRegionSpec]) -> Vec<CapturedRegion> {
    specs
        .iter()
        .map(|spec| {
            let bytes = rt.space_memory(spec.space).ok().and_then(|memory| {
                ByteRange::new(GuestAddr::new(spec.addr), spec.size)
                    .and_then(|range| memory.read(range))
            });
            match bytes {
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
#[path = "tests/capture_tests.rs"]
mod capture_tests;
