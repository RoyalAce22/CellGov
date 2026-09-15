//! Benchmarks from the partial-order-reduction literature, held
//! against the counts their papers state.
//!
//! Every other count in this crate is one we derived, so it checks the
//! search against our own reading of the algorithm. A reading that is
//! wrong twice in the same way passes all of them. These come from
//! outside the project, so a mismatch here is a defect in our
//! implementation rather than a new result.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_explore::{explore_optimal, ExplorationConfig};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;

/// One shared variable per address, spaced past the four bytes each
/// access touches so distinct variables never overlap.
const X: u64 = 0;
const Y: u64 = 8;
const Z: u64 = 16;
const G: u64 = 24;

fn store(addr: u64, value: u32) -> Vec<FakeOp> {
    vec![FakeOp::LoadImm(value), FakeOp::SharedStore { addr, len: 4 }]
}

fn load(addr: u64) -> Vec<FakeOp> {
    vec![FakeOp::SharedLoad { addr, len: 4 }]
}

/// Build a runtime whose units run `programs`, each terminated.
fn program(programs: Vec<Vec<FakeOp>>) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(1), 400);
    for mut ops in programs {
        ops.push(FakeOp::End);
        rt.register_unit_with(|id| FakeIsaUnit::new(id, ops.clone()));
    }
    rt
}

/// Equivalence classes the optimal search covers.
fn classes(make: fn() -> Runtime) -> usize {
    let result = explore_optimal(
        make,
        &ExplorationConfig {
            max_schedules: 100_000,
            max_steps_per_run: 10_000,
        },
    );
    result
        .classes_explored
        .expect("the search covered every class")
}

/// `Abdulla2017`'s writer-readers program: one writer of `x` and two
/// units that read an unshared variable and then `x`.
fn writer_readers() -> Runtime {
    program(vec![
        store(X, 1),
        [load(Y), load(X)].concat(),
        [load(Z), load(X)].concat(),
    ])
}

/// Three accesses to `x` order six ways, and the two that differ only
/// in the order of the reads are the same trace, so four remain
/// [Abdulla2017 p:42:5 s:2].
#[test]
fn the_writer_readers_program_has_the_published_four_classes() {
    assert_eq!(classes(writer_readers), 4);
}

/// `Aronis2018`'s writers program: two units that each write `x` and
/// then `y`.
fn writers() -> Runtime {
    program(vec![
        [store(X, 1), store(Y, 1)].concat(),
        [store(X, 2), store(Y, 2)].concat(),
    ])
}

/// The order of the two writes to `x` and the order of the two writes
/// to `y` are independent choices, so four of the six interleavings
/// are distinct traces [Aronis2018 p:231 s:2].
#[test]
fn the_writers_program_has_the_published_four_classes() {
    assert_eq!(classes(writers), 4);
}

/// `Aronis2018`'s second program: two writers of `x` and a reader that
/// checks it.
fn two_writers_and_a_reader() -> Runtime {
    program(vec![store(X, 1), store(X, 2), load(X)])
}

/// Every pair of the three accesses interferes, so all `3! = 6`
/// interleavings are distinct traces [Aronis2018 p:231 s:2].
///
/// The paper reaches two with observers, which decide interference by
/// whether a write is ever read. This search has no observer
/// reduction, so six is the number to expect, and it is the number
/// that moves if one ever lands.
#[test]
fn the_two_writers_and_a_reader_program_has_the_published_six_classes() {
    assert_eq!(classes(two_writers_and_a_reader), 6);
}

/// `Abdulla2024`'s example: four units over `g`, `x`, `y` and `z`.
fn four_units_over_four_variables() -> Runtime {
    program(vec![
        store(X, 1),
        [store(Y, 1), store(Z, 1)].concat(),
        [store(G, 1), load(Y), load(X)].concat(),
        [load(Y), load(Z), load(X)].concat(),
    ])
}

/// The paper states no class count for this program -- it uses it to
/// draw an exploration tree -- so the number here is ours, recorded as
/// a regression pin rather than an external check.
///
/// Its evaluation benchmarks are the external numbers, and those are
/// not portable: they are parametric C programs over a shared-memory
/// model with read-modify-writes and locks, which the fake ISA has no
/// opcode for.
#[test]
fn the_four_unit_program_is_ported_without_a_published_count() {
    assert_eq!(classes(four_units_over_four_variables), 32);
}
