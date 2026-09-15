//! The program that separates source-DPOR from optimal DPOR
//! [Nguyen2018 p:2 s:1].
//!
//! `n` writers each store to their own address. A counter unit writes
//! `1..n` to one shared cell. A master reads that cell and stores to
//! the writer address the value it read names. So the counter decides
//! which writer the master interferes with, and the two interferences
//! are entangled: the order that reverses the first decides the second.
//!
//! The program has `2n` equivalence classes. A search that reverses
//! each race on its own, without carrying the sequence that reached it,
//! tries the master against every writer and explores `O(2^n)`
//! executions.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::{explore_backtrack, explore_optimal, ExplorationConfig};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// Where the counter cell lives, clear of the writer addresses.
const COUNTER: u64 = 0;
/// First writer address; writer `k` owns `WRITERS + k * STRIDE`.
const WRITERS: u64 = 64;
const STRIDE: u64 = 8;

/// The figure's program for `n` writers.
fn entangled(n: u32) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(512), Budget::new(1), 400);
    for index in 0..n {
        rt.register_unit_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(7 + index),
                    FakeOp::SharedStore {
                        addr: WRITERS + u64::from(index) * STRIDE,
                        len: 4,
                    },
                    FakeOp::End,
                ],
            )
        });
    }
    let counter: Vec<FakeOp> = (1..n)
        .flat_map(|value| {
            [
                FakeOp::LoadImm(value),
                FakeOp::SharedStore {
                    addr: COUNTER,
                    len: 4,
                },
            ]
        })
        .chain([FakeOp::End])
        .collect();
    rt.register_unit_with(|id| FakeIsaUnit::new(id, counter.clone()));
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::SharedLoad {
                    addr: COUNTER,
                    len: 4,
                },
                FakeOp::SharedStoreIndexed {
                    base: WRITERS,
                    stride: STRIDE,
                    len: 4,
                },
                FakeOp::End,
            ],
        )
    });
    rt
}

/// Executions each search costs, baseline included.
fn cost(n: u32) -> (usize, usize) {
    let config = ExplorationConfig {
        max_schedules: 100_000,
        max_steps_per_run: 10_000,
    };
    let optimal = explore_optimal(|| entangled(n), &config);
    let scaffold = explore_backtrack(|| entangled(n), &config);
    assert!(
        !optimal.bounds_hit,
        "n={n}: the optimal search covered every class",
    );
    (optimal.schedules.len() + 1, scaffold.schedules.len() + 1)
}

/// The optimal search costs one execution per class, and the paper
/// gives the class count: `2n`, six for three writers
/// [Nguyen2018 p:2 s:1].
#[test]
fn the_optimal_search_costs_the_published_class_count() {
    for n in 2..=5 {
        let (optimal, _) = cost(n);
        assert_eq!(
            optimal,
            2 * n as usize,
            "n={n}: the program has 2n classes and the search runs one execution each",
        );
    }
}

/// The scaffold pays for the entanglement and the optimal search does
/// not.
///
/// `Nguyen2018` states the gap as `O(2^n)` against source-DPOR, which
/// is `Abdulla2017`'s Algorithm 1. The scaffold here is the older
/// backtrack-set algorithm and it dedupes the prefixes it owes, so on
/// this program it costs `7n` rather than `2^n`.
#[test]
fn the_scaffold_costs_more_than_the_optimal_search_on_every_size() {
    let mut optimal_costs = Vec::new();
    let mut scaffold_costs = Vec::new();
    for n in 2..=5 {
        let (optimal, scaffold) = cost(n);
        assert!(
            scaffold > optimal,
            "n={n}: the scaffold explores more than one execution per class",
        );
        optimal_costs.push(optimal);
        scaffold_costs.push(scaffold);
    }
    assert_eq!(optimal_costs, vec![4, 6, 8, 10]);
    assert_eq!(scaffold_costs, vec![7, 14, 21, 28]);
}
