//! A depth the runtime resolved by warping still owes its branches.
//!
//! Where no unit is runnable, the search does not pick: the runtime
//! warps guest time, fires what is due, and runs whatever that wakes.
//! The depth still carries a wakeup tree, and a race the execution
//! reports can ask that depth for a reversal it cannot deliver. The
//! case here is what the depth does with that branch: it drops the
//! branch and counts the drop, rather than re-arming one no replay of
//! the prefix can take.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
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
/// runtime warps to it. The step that warp schedules is the parked
/// unit's write, which races with both writers it then releases. So
/// the warp depth is where those races ask for their reversals, and it
/// is the one depth that cannot give them.
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

/// The search finishes, and says it did not cover everything.
///
/// A race can name a warp depth as the place to reverse it, and the
/// search can reverse nothing there. The runtime picks, and it picks
/// the same unit on every replay of the prefix. So the depth drops the
/// branch and counts it, and that count withdraws the class total.
#[test]
fn a_warp_depth_refuses_a_reversal_it_cannot_deliver() {
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

    // The bug this guards recorded one execution until the cap, so a
    // record count well under the cap is the claim.
    assert!(
        result.schedules.len() < 32,
        "the search owed a handful of reversals, not a runaway: {} records",
        result.schedules.len(),
    );

    assert_eq!(
        result.classes_explored, None,
        "a reversal the search dropped is cover it cannot claim",
    );
}
