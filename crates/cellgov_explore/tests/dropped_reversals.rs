//! A depth with one runnable unit drops the reversals naming another.
//!
//! `choose` walks the branches a depth holds and takes the least one
//! whose unit can run there. Where the depth has a single runnable unit,
//! it retires and counts every branch naming another unit, and one
//! counted drop withdraws the run's class total.
//!
//! `warp_then_contend` reaches such a depth. No assertion here turns on
//! the warp after it: `warp_two_wakes.rs` covers the warp depth, where
//! the search delivers its alternate, and the crate's
//! `warp_retire_tests` reads the two sites apart over this same
//! workload.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_exec::UnitStatus;
use cellgov_explore::{explore_optimal, ExplorationConfig};
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
/// unit's write. Each wake that write makes releases a writer, so it
/// races with the wait that parked that writer. The last of those waits
/// took its depth's only runnable unit, with the other two parked on
/// their own barriers. The reversal its race asks for names one of them.
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

/// A depth left alone because the other units finished would say
/// nothing, so the parked count carries the premise beside the runnable
/// one. Both are read before the step, so a warp, where nothing is
/// runnable, is not one of these.
#[test]
fn a_depth_holds_one_runnable_unit_with_the_rest_parked() {
    let mut rt = warp_then_contend();
    let mut singleton_depths = 0usize;
    for _ in 0..64 {
        let registry = rt.registry();
        let mut runnable = 0usize;
        let mut parked = 0usize;
        for id in registry.ids() {
            match registry.effective_status(id) {
                Some(UnitStatus::Runnable) => runnable += 1,
                Some(UnitStatus::Blocked) => parked += 1,
                Some(UnitStatus::Faulted | UnitStatus::Finished) | None => {}
            }
        }
        let alone = runnable == 1 && parked == 2;
        let Ok(step) = rt.step() else { break };
        if alone {
            singleton_depths += 1;
        }
        rt.commit_step(&step.result, &step.effects)
            .expect("no step of this workload refuses its commit");
    }
    assert!(
        singleton_depths > 0,
        "no depth ran its only runnable unit with the other two parked, so no \
         branch here names a unit that cannot run",
    );
}

#[test]
fn a_depth_with_one_runnable_unit_drops_what_it_cannot_deliver() {
    let cap = 1_000;
    let result = explore_optimal(
        warp_then_contend,
        &ExplorationConfig {
            max_schedules: cap,
            max_steps_per_run: 10_000,
        },
    );
    assert!(
        !result.baseline_stop.is_truncated(),
        "the baseline has to reach the end for the search to owe anything: {}",
        result.baseline_stop,
    );
    assert!(
        result.schedules.len() < cap,
        "the search ran to its cap, which is what a depth re-arming a branch it \
         cannot take looks like",
    );
    assert!(
        !result.bounds_hit,
        "no bound stopped this search: it ran out of branches",
    );

    assert!(
        result.schedules.len() < 32,
        "the search owed a handful of reversals, not a runaway: {} records",
        result.schedules.len(),
    );

    assert_eq!(
        result.classes_explored, None,
        "a reversal the search dropped is cover it cannot claim",
    );
    // Without this the empty class total above reads the same as a
    // bound's.
    assert!(
        result.reversals_dropped > 0,
        "the depth dropped a branch, and the count is what says so",
    );
}
