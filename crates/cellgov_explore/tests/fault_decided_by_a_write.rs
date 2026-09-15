//! A schedule that decides whether a unit faults claims nothing.
//!
//! One unit's fault turns on a byte another unit writes, so the order
//! of the two decides whether the fault happens. Hard rule 4 discards
//! the faulted batch, the read that decided the fault goes with it,
//! and the driver records no event for the step. The relation is
//! therefore never asked about that step: there is no event to ask
//! about.
//!
//! A faulted run is truncated, and that withdraws every claim measured
//! against it. So the search reports no verdict rather than calling
//! the workload stable over the orders it did reach.
//!
//! Two workloads are here, because which order a search reaches first
//! is no part of the argument. Where the read commits, the relation
//! owes the reversal that reaches the fault. Where every order the
//! search reaches faults, the truncation carries the whole answer.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_effects::FaultKind;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp, FAKE_FAULT_CODE};
use cellgov_explore::classify::OutcomeClass;
use cellgov_explore::observer::observe_decisions;
use cellgov_explore::util::{run_to_stall, StopReason};
use cellgov_explore::{explore_optimal, ExplorationConfig};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

/// The byte the fault turns on. Guest memory starts zeroed, so the
/// faulter faults unless the writer went first.
const GATE: u64 = 0;

/// The unit whose fault the gate byte decides, in the workloads that
/// register it second.
const FAULTER: cellgov_event::UnitId = cellgov_event::UnitId::new(1);

/// One unit writes the gate byte; another faults while it reads zero.
fn a_write_decides_a_fault() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 200);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(1),
                FakeOp::SharedStore { addr: GATE, len: 1 },
                FakeOp::End,
            ],
        )
    });
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(id, vec![FakeOp::FaultIfZero { addr: GATE }, FakeOp::End])
    });
    rt
}

/// The same two programs with the faulter holding the lower unit id.
///
/// The search takes the lowest runnable id at each depth of its first
/// execution, so the faulter goes first and reads the gate at zero.
/// No order the search reaches commits the read.
fn the_faulter_holds_the_lower_id() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 200);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(id, vec![FakeOp::FaultIfZero { addr: GATE }, FakeOp::End])
    });
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(1),
                FakeOp::SharedStore { addr: GATE, len: 1 },
                FakeOp::End,
            ],
        )
    });
    rt
}

/// The gate byte already set, so no order faults.
fn the_same_workload_past_the_gate() -> Runtime {
    let mut rt = a_write_decides_a_fault();
    rt.place_bytes(
        cellgov_core::AddressSpaceId::BOOT,
        ByteRange::new(GuestAddr::new(GATE), 1).unwrap(),
        &[1],
    )
    .unwrap();
    rt
}

#[test]
fn the_workload_faults_under_one_order_and_not_the_other() {
    let mut faults = a_write_decides_a_fault();
    assert_eq!(
        run_to_stall(&mut faults, 200),
        StopReason::Faulted(FaultKind::Guest(FAKE_FAULT_CODE)),
        "round-robin reaches the faulter while the gate is still zero",
    );

    let mut runs = the_same_workload_past_the_gate();
    assert_eq!(
        run_to_stall(&mut runs, 200),
        StopReason::Stalled,
        "the same two programs finish once the gate byte is set, so the fault \
         is the schedule's and not the workload's",
    );
}

/// The faulted step leaves no event behind.
///
/// A discarded batch published nothing, so recording an event for it
/// would give the relation a footprint naming no committed access.
/// The driver stops at the fault instead, so no pair involving that
/// step reaches the independence test.
#[test]
fn a_faulted_step_records_no_event() {
    let mut rt = a_write_decides_a_fault();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Faulted(FaultKind::Guest(FAKE_FAULT_CODE)));
    assert!(
        log.points().iter().all(|point| point.chosen != FAULTER),
        "the run stops at the fault, so the faulter contributes no event",
    );
    assert_eq!(
        log.len(),
        1,
        "round-robin commits the writer's first step, then the faulter's step \
         faults and claims no point of its own",
    );
}

#[test]
fn a_schedule_decided_fault_withdraws_the_verdict() {
    let result = explore_optimal(a_write_decides_a_fault, &ExplorationConfig::default());
    assert_eq!(
        result.baseline_stop,
        StopReason::Stalled,
        "the search's own first order runs the writer first, so the baseline \
         never sees the gate at zero",
    );

    // The write and the read conflict, so the relation owes their
    // reversal. Running it is what reaches the fault.
    let reversal = result
        .schedules
        .iter()
        .find(|record| record.alternate_choice == FAULTER)
        .expect("the search owes the order that puts the faulter first");
    assert_eq!(
        reversal.stop,
        StopReason::Faulted(FaultKind::Guest(FAKE_FAULT_CODE)),
        "the reversed order reads the gate at zero and faults",
    );
    assert!(reversal.truncated);
    assert_eq!(result.schedules_truncated, 1);
    assert_eq!(
        result.schedules_refused, 0,
        "the guest's own step failed; the model refused nothing",
    );

    assert_eq!(
        result.classes_explored, None,
        "a truncated alternate is a class the search did not cover",
    );
    assert_eq!(
        result.outcome,
        OutcomeClass::Inconclusive,
        "one order faults, so the search refuses to call the workload stable",
    );
}

/// Truncation, not the relation, is what withdraws the verdict where
/// every order the search reaches faults.
///
/// The read that decides the fault reaches no execution here, so the
/// relation is asked about no pair and concludes nothing. What keeps
/// the verdict honest is that the one run faulted: a faulted run is
/// truncated, and a truncated baseline withdraws every claim measured
/// against its hash.
#[test]
fn a_baseline_that_faults_first_leaves_the_relation_nothing_to_read() {
    let result = explore_optimal(
        the_faulter_holds_the_lower_id,
        &ExplorationConfig::default(),
    );
    assert_eq!(
        result.baseline_stop,
        StopReason::Faulted(FaultKind::Guest(FAKE_FAULT_CODE)),
        "the lowest runnable id goes first, so the faulter reads the gate at zero",
    );
    assert_eq!(
        result.baseline_steps, 0,
        "the faulted step is no event, so the baseline committed nothing",
    );
    assert!(
        result.schedules.is_empty(),
        "a truncated baseline reads no races, so it owes no alternate",
    );
    assert_eq!(
        result.outcome,
        OutcomeClass::Inconclusive,
        "the search reached one order, it faulted, and that withdraws the verdict",
    );
    assert_eq!(result.classes_explored, None);
}

/// The positive control: past the gate, the same two programs get a
/// verdict.
///
/// Without this, every assertion above would pass for a search that
/// called everything inconclusive.
#[test]
fn the_same_workload_past_the_gate_reaches_a_verdict() {
    let result = explore_optimal(
        the_same_workload_past_the_gate,
        &ExplorationConfig::default(),
    );
    assert_eq!(result.baseline_stop, StopReason::Stalled);
    assert!(!result.baseline_stop.is_truncated());
    assert_ne!(
        result.outcome,
        OutcomeClass::Inconclusive,
        "the gate byte is the only difference, so this run has to answer",
    );
    assert!(result.classes_explored.is_some());
}
