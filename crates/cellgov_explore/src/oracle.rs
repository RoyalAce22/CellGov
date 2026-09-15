//! Exploration wrapper that also captures named memory regions from
//! each run for comparison against an oracle.

use crate::classify::ExplorationResult;
use crate::config::ExplorationConfig;
use crate::optimal::explore_optimal_observed;
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
    /// False when this run's committed memory holds nothing at the
    /// spec's range:
    ///
    /// - a space the run never created;
    /// - an unmapped address;
    /// - an overflowing range.
    pub resolved: bool,
}

/// Memory snapshot from one explored schedule.
#[derive(Debug, Clone)]
pub struct ScheduleSnapshot {
    /// Final observable hash ([`crate::classify::OBSERVABLE`]).
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
    /// Snapshot from each explored alternate, in exploration order.
    pub alternates: Vec<ScheduleSnapshot>,
}

/// Like [`crate::explore()`] but also captures named regions from every
/// run.
///
/// Returns `None` if the baseline has no branching points. A baseline
/// that stopped short withdraws every divergence claim, exactly as in
/// [`crate::explore()`].
pub fn explore_with_regions<F>(
    make_runtime: F,
    config: &ExplorationConfig,
    regions: &[MemoryRegionSpec],
) -> Option<OracleExplorationResult>
where
    F: FnMut() -> Runtime,
{
    let mut baseline_regions = Vec::new();
    let mut alternates = Vec::new();
    let exploration = explore_optimal_observed(make_runtime, config, |rt, is_baseline| {
        let captured = extract_regions(rt, regions);
        if is_baseline {
            baseline_regions = captured;
        } else {
            alternates.push(ScheduleSnapshot {
                memory_hash: rt.observable_hash(),
                regions: captured,
            });
        }
    });
    if exploration.total_branching_points == 0 {
        return None;
    }
    // Nothing in the type system pairs these two: the callback and the
    // schedule record fire from separate tests. One without the other
    // shifts every later region against the record beside it.
    debug_assert_eq!(
        alternates.len(),
        exploration.schedules.len(),
        "the observation callback and the schedule record must fire together",
    );

    let baseline = ScheduleSnapshot {
        memory_hash: exploration.baseline_hash,
        regions: baseline_regions,
    };
    Some(OracleExplorationResult {
        exploration,
        baseline,
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
