//! The stop a step's fault ends a drive with: a store a reserved region
//! refused, or any other fault.

use super::*;
use cellgov_exec::{DataAccessFault, ExecutionStepResult, LocalDiagnostics, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

fn faulted(data_access: Option<DataAccessFault>) -> ExecutionStepResult {
    let mut diag = LocalDiagnostics::empty();
    diag.faulting_ea = Some(0xC000_0040);
    diag.data_access = data_access;
    ExecutionStepResult {
        yield_reason: YieldReason::Fault,
        consumed_cost: InstructionCost::ZERO,
        local_diagnostics: diag,
        fault: Some(FaultKind::Guest(7)),
        syscall_args: None,
    }
}

fn runtime() -> Runtime {
    Runtime::new(GuestMemory::new(64), Budget::new(1), 100)
}

#[test]
fn a_store_a_region_refused_stops_as_a_reserved_store() {
    let step = faulted(Some(DataAccessFault {
        store: true,
        refused_by: Some("rsx"),
    }));
    assert_eq!(
        post_commit_stop(&runtime(), &step),
        Some(StopReason::ReservedStore {
            addr: 0xC000_0040,
            region: "rsx",
        })
    );
}

#[test]
fn a_store_to_an_unmapped_address_stops_as_a_fault() {
    let step = faulted(Some(DataAccessFault {
        store: true,
        refused_by: None,
    }));
    assert_eq!(
        post_commit_stop(&runtime(), &step),
        Some(StopReason::Faulted(FaultKind::Guest(7)))
    );
}

#[test]
fn a_load_stops_as_a_fault() {
    let step = faulted(Some(DataAccessFault {
        store: false,
        refused_by: Some("rsx"),
    }));
    assert_eq!(
        post_commit_stop(&runtime(), &step),
        Some(StopReason::Faulted(FaultKind::Guest(7)))
    );
}

#[test]
fn a_step_with_no_fault_does_not_stop() {
    let mut step = faulted(Some(DataAccessFault {
        store: true,
        refused_by: Some("rsx"),
    }));
    step.fault = None;
    assert_eq!(post_commit_stop(&runtime(), &step), None);
}
