//! A schedule that did not finish does not report as one that did.
//!
//! `Runtime::step` decides when a run is over, and an empty runnable
//! set is not that decision; `run_to_stall` carries the rule. These
//! workloads catch two ways a driver breaks it:
//!
//! - It answers for the empty set itself, and calls a parked schedule
//!   finished.
//! - It ignores a fault, and calls a faulted schedule finished two
//!   steps later.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_effects::FaultKind;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp, FAKE_FAULT_CODE};
use cellgov_explore::util::run_to_stall;
use cellgov_explore::{observe_decisions, ExplorationConfig, OutcomeClass, StopClass, StopReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_testkit::world::DmaSubmitter;
use cellgov_time::Budget;

/// One unit that puts a DMA and parks until it completes. Nothing else
/// is runnable, so the runtime must warp to fire the transfer.
fn parked_on_a_transfer() -> Runtime {
    let src = ByteRange::new(GuestAddr::new(0), 4).unwrap();
    let dst = ByteRange::new(GuestAddr::new(128), 4).unwrap();
    let mut rt = Runtime::new(GuestMemory::new(256), Budget::new(1), 200);
    rt.register_unit_with(|id| DmaSubmitter::new(id, src, dst, vec![0xde, 0xad, 0xbe, 0xef]));
    rt
}

/// A unit that faults on its second step, and one that finishes.
fn one_unit_faults() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 200);
    rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![FakeOp::LoadImm(1), FakeOp::Fault]));
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::LoadImm(2),
                FakeOp::SharedStore { addr: 0, len: 4 },
                FakeOp::End,
            ],
        )
    });
    rt
}

#[test]
fn a_run_parked_on_a_transfer_finishes_the_workload() {
    let mut rt = parked_on_a_transfer();
    let stop = run_to_stall(&mut rt, 200);
    assert_eq!(
        stop,
        StopReason::Stalled,
        "the warp fires the transfer and the issuer runs on",
    );
    assert!(
        rt.steps_taken() > 1,
        "the park is not the end of the workload: the unit woke and ran again",
    );
    let read = rt
        .memory()
        .read(ByteRange::new(GuestAddr::new(128), 4).unwrap())
        .unwrap();
    assert_eq!(read, &[0xde, 0xad, 0xbe, 0xef], "the transfer landed");
}

#[test]
fn the_observer_drives_a_parked_run_the_same_way() {
    let mut rt = parked_on_a_transfer();
    let (log, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Stalled);
    assert!(
        log.len() > 1,
        "the observer recorded the steps after the warp, not just the one before it",
    );
}

#[test]
fn a_fault_stops_the_run_and_is_not_a_stall() {
    let mut rt = one_unit_faults();
    let stop = run_to_stall(&mut rt, 200);
    assert_eq!(
        stop,
        StopReason::Faulted(FaultKind::Guest(FAKE_FAULT_CODE)),
        "the kind the unit raised reaches the stop, so no refused commit stands in for it",
    );
    assert!(stop.is_truncated(), "a faulted run answers for nothing");
    assert_eq!(
        stop.class(),
        StopClass::Fault,
        "the guest's own step failed, which is not the model refusing one",
    );
}

#[test]
fn a_fault_reads_as_a_fault_wherever_a_report_prints_it() {
    // The CLI's drive to a window's start prints "fault" from a stop
    // enum of its own, so one boot carries two names for one stop unless
    // the class label is the same word.
    assert_eq!(StopClass::Fault.label(), "fault");
    assert_ne!(StopClass::Fault.label(), StopClass::Refusal.label());
}

#[test]
fn the_observer_stops_on_a_fault_too() {
    let mut rt = one_unit_faults();
    let (_, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Faulted(FaultKind::Guest(FAKE_FAULT_CODE)));
}

/// One unit that waits for a message nobody sends.
fn parked_for_good() -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(1), 200);
    let mailbox = rt.mailbox_registry_mut().register(4);
    rt.register_unit_with(|id| {
        FakeIsaUnit::new(
            id,
            vec![
                FakeOp::MailboxRecv {
                    mailbox: mailbox.raw(),
                },
                FakeOp::End,
            ],
        )
    });
    rt
}

#[test]
fn a_run_parked_with_no_wake_source_is_reported_as_one() {
    let mut rt = parked_for_good();
    let stop = run_to_stall(&mut rt, 200);
    assert_eq!(stop, StopReason::Deadlocked);
    assert!(
        !stop.is_truncated(),
        "no schedule reaches further, so the run answers for itself",
    );
    assert_eq!(
        stop.class(),
        StopClass::Blocked,
        "a deadlock ends the run without the workload running itself out",
    );
    assert_ne!(
        stop,
        StopReason::Stalled,
        "the report separates a park nothing can wake from a workload that finished",
    );
}

#[test]
fn the_observer_reports_a_deadlock_the_same_way() {
    let mut rt = parked_for_good();
    let (_, stop) = observe_decisions(&mut rt);
    assert_eq!(stop, StopReason::Deadlocked);
}

#[test]
fn a_faulted_baseline_answers_for_nothing() {
    let result = cellgov_explore::explore_window(one_unit_faults, &ExplorationConfig::default());
    assert_eq!(
        result.baseline_stop,
        StopReason::Faulted(FaultKind::Guest(FAKE_FAULT_CODE))
    );
    assert_eq!(result.outcome, OutcomeClass::Inconclusive);
    assert!(result.bounds_hit);
    assert_eq!(result.classes_explored, None);
}

/// A point built from the set read before a warp names a unit the
/// scheduler did not offer, at a step where a choice existed.
#[test]
fn every_point_records_a_set_its_choice_belongs_to() {
    for make in [parked_on_a_transfer, one_unit_faults] {
        let mut rt = make();
        let (log, _) = observe_decisions(&mut rt);
        assert!(!log.is_empty());
        for point in log.points() {
            assert!(
                point.runnable.contains(&point.chosen),
                "step {}: chose {:?} from {:?}",
                point.step,
                point.chosen,
                point.runnable,
            );
        }
    }
}
