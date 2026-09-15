//! The two searches over one workload, in one process.
//!
//! The bounded enumerator tries each alternate at each branching point
//! independently and prunes a pair of units that never conflict. The
//! backtrack-set search walks races instead. Both reach the same set
//! of final memory hashes, which is the property a reduction can
//! silently lose. They are free to disagree on how many executions it
//! cost.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::{explore_backtrack, explore_window, ExplorationConfig, ExplorationResult};
use cellgov_mem::GuestMemory;
use cellgov_testkit::fixtures;
use cellgov_time::Budget;
use std::collections::BTreeSet;

/// Every final memory hash a search reached and can answer for.
///
/// A truncated execution's hash covers a prefix of the workload, so it
/// is not an outcome either search claims.
fn outcomes(result: &ExplorationResult) -> BTreeSet<u64> {
    let mut set = BTreeSet::new();
    if !result.baseline_stop.is_truncated() {
        set.insert(result.baseline_hash);
    }
    for record in &result.schedules {
        if !record.truncated {
            set.insert(record.memory_hash);
        }
    }
    set
}

/// Run both searches over `make_runtime` and hold their outcome sets
/// equal, at `expected` hashes each.
///
/// The count keeps the comparison from passing on two sets that
/// collapsed the same way:
///
/// - a truncated baseline withdraws every record, which leaves both
///   searches with the empty set;
/// - a reduction both searches share leaves both with the baseline
///   hash alone.
///
/// Returns the executions each one cost, baseline included, so a caller
/// can record the gap.
fn agree_on(name: &str, expected: usize, make_runtime: fn() -> Runtime) -> (usize, usize) {
    let config = ExplorationConfig::default();
    let enumerated = explore_window(make_runtime, &config);
    let backtracked = explore_backtrack(make_runtime, &config);
    let reached = outcomes(&backtracked);
    assert_eq!(
        outcomes(&enumerated),
        reached,
        "{name}: the two searches reach different final memory hashes",
    );
    assert_eq!(
        reached.len(),
        expected,
        "{name}: the searches agree on a different number of final memory hashes",
    );
    assert_eq!(
        enumerated.outcome, backtracked.outcome,
        "{name}: the two searches classify the workload differently",
    );
    (
        enumerated.schedules.len() + 1,
        backtracked.schedules.len() + 1,
    )
}

fn fake_isa_program(units: Vec<Vec<FakeOp>>) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 200);
    for program in units {
        rt.register_unit_with(|id| FakeIsaUnit::new(id, program.clone()));
    }
    rt
}

fn two_writers_one_address() -> Runtime {
    fake_isa_program(vec![
        vec![
            FakeOp::LoadImm(0xAA),
            FakeOp::SharedStore { addr: 0, len: 4 },
            FakeOp::End,
        ],
        vec![
            FakeOp::LoadImm(0xBB),
            FakeOp::SharedStore { addr: 0, len: 4 },
            FakeOp::End,
        ],
    ])
}

fn three_writers_one_address() -> Runtime {
    fake_isa_program(vec![
        vec![
            FakeOp::LoadImm(0xAA),
            FakeOp::SharedStore { addr: 0, len: 4 },
            FakeOp::End,
        ],
        vec![
            FakeOp::LoadImm(0xBB),
            FakeOp::SharedStore { addr: 0, len: 4 },
            FakeOp::End,
        ],
        vec![
            FakeOp::LoadImm(0xCC),
            FakeOp::SharedStore { addr: 0, len: 4 },
            FakeOp::End,
        ],
    ])
}

fn two_writers_disjoint() -> Runtime {
    fake_isa_program(vec![
        vec![
            FakeOp::LoadImm(0xAA),
            FakeOp::SharedStore { addr: 0, len: 4 },
            FakeOp::End,
        ],
        vec![
            FakeOp::LoadImm(0xBB),
            FakeOp::SharedStore { addr: 8, len: 4 },
            FakeOp::End,
        ],
    ])
}

fn reader_and_writer() -> Runtime {
    fake_isa_program(vec![
        vec![
            FakeOp::LoadImm(0xAA),
            FakeOp::SharedStore { addr: 0, len: 4 },
            FakeOp::End,
        ],
        vec![
            FakeOp::SharedLoad { addr: 0, len: 4 },
            FakeOp::SharedStore { addr: 8, len: 4 },
            FakeOp::End,
        ],
    ])
}

/// Where the expected hash counts come from:
///
/// - the store order decides the last writer over one address;
/// - disjoint stores both land, whatever the order;
/// - the reader takes the writer's byte or the zero before it.
#[test]
fn the_two_searches_agree_on_the_fake_isa_workloads() {
    agree_on("two writers, one address", 2, two_writers_one_address);
    agree_on("three writers, one address", 3, three_writers_one_address);
    agree_on("two writers, disjoint", 1, two_writers_disjoint);
    agree_on("reader and writer", 2, reader_and_writer);
}

/// Every fixture reaches one final memory hash. Five hold one unit, or
/// units whose footprints never conflict, so neither search replays
/// anything. The two that race reach the same memory by every order.
///
/// The fake-ISA workloads above carry the case where the answer is
/// more than one hash.
#[test]
fn the_two_searches_agree_on_the_testkit_fixtures() {
    agree_on("round robin", 1, || {
        fixtures::round_robin_fairness_scenario(3, 3).build_runtime()
    });
    agree_on("dma block unblock", 1, || {
        fixtures::dma_block_unblock_scenario().build_runtime()
    });
    agree_on("mailbox send", 1, || {
        fixtures::mailbox_send_scenario(2).build_runtime()
    });
    agree_on("signal update", 1, || {
        fixtures::signal_update_scenario(2).build_runtime()
    });
    agree_on("fake isa", 1, || {
        fixtures::fake_isa_scenario().build_runtime()
    });

    // Both searches replay off a race in these two fixtures, so their
    // agreement covers more than one execution each.
    let (enumerated, backtracked) = agree_on("write conflict", 1, || {
        fixtures::write_conflict_scenario(3).build_runtime()
    });
    assert!(
        enumerated > 1 && backtracked > 1,
        "write conflict: both searches replay off the fixture's race \
         ({enumerated} and {backtracked} executions)",
    );

    let (enumerated, backtracked) = agree_on("mailbox roundtrip", 1, || {
        fixtures::mailbox_roundtrip_scenario(7).build_runtime()
    });
    assert!(
        enumerated > 1 && backtracked > 1,
        "mailbox roundtrip: both searches replay off the fixture's race \
         ({enumerated} and {backtracked} executions)",
    );
}

/// Three writers over one address commit three conflicting stores. The
/// other six steps touch no shared state, so the equivalence classes
/// are the orders of those three stores. That gives `3! = 6` traces
/// and three outcomes, one per unit that can commit the last write.
///
/// Neither search reaches six. The assertion pins what each costs
/// today, so an improvement appears as a moved number.
#[test]
fn the_cost_of_three_writers_is_recorded() {
    let (enumerated, backtracked) = agree_on("three writers", 3, three_writers_one_address);
    assert_eq!(
        (enumerated, backtracked),
        (16, 10),
        "executions each search costs, baseline included, against 6 traces",
    );
}
