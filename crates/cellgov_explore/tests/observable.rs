//! Declared regions never narrow the verdict.
//!
//! The verdict compares one observable, the committed memory of every
//! address space and every SPU's local store at the end of the run.
//! `explore_with_regions` adds a second comparison against an oracle.
//! Whatever regions a caller declares, and whether it declares any, the
//! verdict is the one `explore_window` gives for the same workload.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_explore::{
    explore_window, explore_with_regions, ExplorationConfig, ExplorationResult, MemoryRegionSpec,
    OutcomeClass,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::{CountingUnit, DmaSubmitter, WritingUnit};
use cellgov_time::Budget;

const STEP_CAP: usize = 200;

/// The one range two schedules leave different: the transfer's
/// destination, which the writer also stores to.
fn destination() -> ByteRange {
    ByteRange::new(GuestAddr::new(128), 4).unwrap()
}

/// A transfer in flight, a writer over its destination, and a unit that
/// only spends ticks; the order of the writer's last store against the
/// landing decides the outcome.
fn workload() -> Runtime {
    let src = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(2), STEP_CAP);
    rt.register_unit_with(|id| {
        DmaSubmitter::new(id, src, destination(), vec![0xde, 0xad, 0xbe, 0xef])
    });
    rt.register_unit_with(|id| WritingUnit::new(id, 3, destination()));
    rt.register_unit_with(|id| CountingUnit::new(id, 8));
    rt
}

fn config() -> ExplorationConfig {
    ExplorationConfig {
        max_schedules: 1_000,
        max_steps_per_run: 10_000,
    }
}

/// The whole result, not the outcome alone: an outcome hides a field
/// that moved (`result_equality.rs`), and a region that changed which
/// schedules ran or what they hashed would still read as the same
/// verdict.
fn exploration_with(regions: &[MemoryRegionSpec]) -> ExplorationResult {
    explore_with_regions(workload, &config(), regions)
        .expect("the workload holds a branching point")
        .exploration
}

/// The premise: the workload is schedule-sensitive on the whole-memory
/// observable, so a narrowed one would have something to hide.
#[test]
fn the_workload_is_schedule_sensitive_on_the_whole_memory() {
    let result = explore_window(workload, &config());
    assert_eq!(result.outcome, OutcomeClass::ScheduleSensitive);
}

#[test]
fn no_declared_region_gives_the_whole_memory_verdict() {
    assert_eq!(exploration_with(&[]), explore_window(workload, &config()));
}

/// A region clear of the bytes the schedules disagree on does not turn
/// the verdict stable: the region is an oracle comparison, not the
/// observable.
#[test]
fn a_region_clear_of_the_divergence_leaves_the_verdict_sensitive() {
    let source_only = MemoryRegionSpec {
        name: "source".into(),
        space: AddressSpaceId::BOOT,
        addr: 0,
        size: 4,
    };
    let r = exploration_with(&[source_only]);
    assert_eq!(r.outcome, OutcomeClass::ScheduleSensitive);
    assert_eq!(r, explore_window(workload, &config()));
}

/// A region that resolves to nothing lands as an unresolved capture and
/// changes the verdict no more than a resolved one does.
#[test]
fn an_unresolved_region_leaves_the_verdict_alone() {
    let nowhere = MemoryRegionSpec {
        name: "nowhere".into(),
        space: AddressSpaceId::new(7),
        addr: 0,
        size: 4,
    };
    let r = explore_with_regions(workload, &config(), &[nowhere])
        .expect("the workload holds a branching point");
    assert!(
        r.baseline.regions.iter().all(|region| !region.resolved),
        "the premise: the region resolves in no run",
    );
    assert_eq!(r.exploration.outcome, OutcomeClass::ScheduleSensitive);
    assert_eq!(r.exploration, explore_window(workload, &config()));
}
