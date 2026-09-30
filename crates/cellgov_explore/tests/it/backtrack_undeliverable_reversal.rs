//! The backtrack search counts a race it cannot turn into a candidate.
//!
//! The search forces the later event's unit at the earlier event. Where
//! that unit cannot run there, it falls back to every other runnable
//! unit. A point that ran the only runnable unit leaves that fallback
//! empty, because every other unit was parked. `warp_then_contend`
//! reaches such a point, so one of its races is owed and undeliverable.
//! `reversals_dropped` says so, and separates this search's zero from
//! the zero of a search that gave nothing up.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::{explore_backtrack, explore_optimal, ExplorationConfig};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// The word every unit writes, so each pair of writes conflicts.
const SHARED: u64 = 0;

/// The transfer's two ends, both clear of [`SHARED`].
const DMA_SRC: u64 = 64;
const DMA_DST: u64 = 128;

/// One unit parks until a transfer completes, then writes the shared
/// word and releases two writers that write it too.
///
/// Nothing is runnable while the transfer is outstanding, so the
/// runtime warps to it, and the step that warp schedules is the parked
/// unit's write. The wake that follows that write releases each writer,
/// so it races with the wait that parked it. A replay that parks the
/// first unit before either writer leaves a step where one writer is
/// the only runnable unit. That wait's race asks for its reversal there.
fn warp_then_contend() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(1), 400);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::DmaPut {
                    src: DMA_SRC,
                    dst: DMA_DST,
                    len: 4,
                },
                FakeOp::Barrier { barrier: 0 },
                FakeOp::SharedStore {
                    addr: SHARED,
                    len: 4,
                },
                FakeOp::Wake { unit: 1 },
                FakeOp::Wake { unit: 2 },
                FakeOp::End,
            ],
        )
    });
    for index in 1..=2u64 {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::Barrier { barrier: index },
                    FakeOp::LoadImm(0xC0 + index as u32),
                    FakeOp::SharedStore {
                        addr: SHARED,
                        len: 4,
                    },
                    FakeOp::End,
                ],
            )
        });
    }
    rt
}

fn run(config_cap: usize) -> cellgov_explore::ExplorationResult {
    explore_backtrack(
        warp_then_contend,
        &ExplorationConfig {
            max_schedules: config_cap,
            max_steps_per_run: 10_000,
        },
    )
}

#[test]
fn the_baseline_runs_itself_out() {
    let result = run(1_000);
    assert!(
        !result.baseline_stop.is_truncated(),
        "the baseline has to reach the end for the search to owe anything: {}",
        result.baseline_stop,
    );
    assert!(
        !result.bounds_hit,
        "no bound stopped this search: it ran out of candidates",
    );
}

#[test]
fn a_race_that_reaches_no_candidate_is_counted() {
    let result = run(1_000);
    assert!(
        result.reversals_dropped > 0,
        "a point that ran the only runnable unit owes a reversal it cannot \
         deliver, and the count is what says so",
    );
    assert!(
        !result.schedules.is_empty(),
        "the count is only worth reading beside the schedules it ran",
    );
}

/// Every replay whose prefix reaches an undeliverable race walks that
/// race again, so the count is per reversal. The shared race walk and
/// the fallback order move both pins.
#[test]
fn the_count_names_reversals_rather_than_walks() {
    let result = run(1_000);
    assert_eq!(
        result.schedules.len(),
        43,
        "the premise: the search re-walks its races across many replays",
    );
    assert_eq!(
        result.reversals_dropped, 5,
        "one per distinct prefix and race, not one per walk",
    );
}

/// Both searches read the same races of the same workload, so a change
/// to the shared race walk moves this too.
#[test]
fn the_optimal_search_still_drops_and_counts_the_same() {
    let result = explore_optimal(
        warp_then_contend,
        &ExplorationConfig {
            max_schedules: 1_000,
            max_steps_per_run: 10_000,
        },
    );
    assert_eq!(
        result.reversals_dropped, 2,
        "the optimal search drops the same two reversals it did before",
    );
    assert_eq!(
        result.schedules.len(),
        1,
        "and records the one schedule it did before",
    );
}
