//! The runtime's stop-state query and restart refuse what they cannot
//! resume.

use cellgov_event::UnitId;
use cellgov_exec::{FakeIsaUnit, FakeOp, RestartError, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_time::Budget;
use cellgov_trace::{TraceReader, TraceRecord};

use super::*;

#[test]
fn an_unknown_unit_has_no_stop_state_and_refuses_a_restart() {
    let mut rt = Runtime::new(GuestMemory::new(4096), Budget::new(1), 1);
    let unknown = UnitId::new(9);
    assert_eq!(rt.unit_stop_registers(unknown), None);
    assert_eq!(rt.restart_unit(unknown), Err(RestartError::UnknownUnit));
}

#[test]
fn a_unit_that_finishes_without_stopping_itself_is_not_traced_and_refuses_a_restart() {
    let mut rt = Runtime::new(GuestMemory::new(4096), Budget::new(1), 1);
    let unit = rt.register_unit_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
    let step = rt.step().expect("the unit runs");
    rt.commit_step(&step.result, &step.effects)
        .expect("the step commits");
    assert_eq!(step.result.yield_reason, YieldReason::Finished);

    assert_eq!(rt.unit_stop_registers(unit), None);
    assert!(!TraceReader::new(rt.trace().bytes())
        .map(|record| record.expect("the runtime's own stream decodes"))
        .any(|record| matches!(record, TraceRecord::UnitStopped { .. })));
    assert_eq!(rt.restart_unit(unit), Err(RestartError::NotStopped));
}
