//! Which of the two sites the search's dropped branches come from;
//! [`Drops`] splits the aggregate [`ExplorationResult::reversals_dropped`]
//! carries.

use super::*;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// The word every unit writes, so each pair of writes conflicts.
const SHARED: u64 = 0;
const DMA_SRC: u64 = 64;
const DMA_DST: u64 = 128;

fn rt_of(programs: &[Vec<FakeOp>]) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(1), 400);
    for program in programs {
        let program = program.clone();
        rt.register_unit_with(move |id| FakeIsaUnit::new(id, program.clone()));
    }
    rt
}

fn store() -> FakeOp {
    FakeOp::SharedStore {
        addr: SHARED,
        len: 4,
    }
}

/// Enqueues a transfer, parks until it lands, then writes the shared
/// word.
///
/// Nothing is runnable while the transfer is outstanding, so the runtime
/// warps to it and the step that warp schedules is this unit's write.
fn waiter(barrier: u64, destination: u64) -> Vec<FakeOp> {
    vec![
        FakeOp::DmaPut {
            src: DMA_SRC,
            dst: destination,
            len: 4,
        },
        FakeOp::Barrier { barrier },
        store(),
        FakeOp::End,
    ]
}

fn drops_over(programs: &[Vec<FakeOp>]) -> Drops {
    search(
        || rt_of(programs),
        &ExplorationConfig {
            max_schedules: 2_000,
            max_steps_per_run: 10_000,
        },
        |_, _| {},
    )
    .1
}

/// `dropped_reversals.rs`'s workload, which that file says drops at
/// `choose` and not at the warp depth its transfer produces.
fn warp_then_contend() -> Vec<Vec<FakeOp>> {
    let mut programs = vec![vec![
        FakeOp::DmaPut {
            src: DMA_SRC,
            dst: DMA_DST,
            len: 4,
        },
        FakeOp::Barrier { barrier: 0 },
        store(),
        FakeOp::Wake { unit: 1 },
        FakeOp::Wake { unit: 2 },
        FakeOp::End,
    ]];
    for barrier in 1..=2u64 {
        programs.push(vec![
            FakeOp::Barrier { barrier },
            FakeOp::LoadImm(0xC0 + barrier as u32),
            store(),
            FakeOp::End,
        ]);
    }
    programs
}

#[test]
fn the_contending_workload_drops_only_at_choose() {
    let drops = drops_over(&warp_then_contend());

    assert_eq!(drops.by_choose, 2, "the depth with one runnable unit");
    assert!(
        drops.at_warp_visits > 0,
        "the transfer's warp depth is what the next reading is about",
    );
    assert_eq!(drops.at_warp, 0, "and the warp depth gave up nothing");
}

#[test]
fn no_warp_workload_gives_up_a_branch_at_its_warp_depth() {
    let cases: Vec<(&str, Vec<Vec<FakeOp>>)> = vec![
        (
            "two waiters",
            vec![waiter(0, DMA_DST), waiter(1, DMA_DST + 8)],
        ),
        (
            "two waiters and a writer",
            vec![
                waiter(0, DMA_DST),
                waiter(1, DMA_DST + 8),
                vec![store(), FakeOp::End],
            ],
        ),
        (
            "a writer ahead of two waiters",
            vec![
                vec![store(), FakeOp::End],
                waiter(0, DMA_DST),
                waiter(1, DMA_DST + 8),
            ],
        ),
        (
            "three waiters",
            vec![
                waiter(0, DMA_DST),
                waiter(1, DMA_DST + 8),
                waiter(2, DMA_DST + 16),
            ],
        ),
        ("contention after a warp", warp_then_contend()),
    ];

    for (name, programs) in cases {
        let drops = drops_over(&programs);
        assert!(
            drops.at_warp_visits > 0,
            "{name} reached no warp depth, so its zero at the warp site is \
             no reading of that site",
        );
        assert!(
            drops.by_choose > 0,
            "{name} gave up nothing at all, so it says nothing about the \
             other site",
        );
        assert_eq!(
            drops.at_warp, 0,
            "{name} gave up a branch at its warp depth"
        );
    }
}
