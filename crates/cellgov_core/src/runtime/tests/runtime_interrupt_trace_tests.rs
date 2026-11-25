//! The trace record of a step that ends at an interrupt boundary.

use super::*;
use crate::runtime::types::RuntimeMode;
use cellgov_exec::InterruptRecord;
use cellgov_trace::{TraceReader, TraceRecord};

/// A unit whose one step costs its budget and yields `yield_reason` and `interrupt`.
#[derive(Clone)]
struct InterruptingUnit {
    id: UnitId,
    done: bool,
    yield_reason: YieldReason,
    interrupt: Option<InterruptRecord>,
}

impl ExecutionUnit for InterruptingUnit {
    type Snapshot = ();
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.done {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.done = true;
        let mut local_diagnostics = LocalDiagnostics::empty();
        local_diagnostics.interrupt = self.interrupt;
        ExecutionStepResult {
            yield_reason: self.yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics,
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) {}
}

const RECORD: InterruptRecord = InterruptRecord {
    vector: 0xC00,
    srr0: 0x0002_0004,
    srr1: 0x8000_0000_0000_1000,
    dsisr: None,
    dar: None,
};

/// The delivered-interrupt records of one step under `mode`.
fn delivered(
    mode: RuntimeMode,
    yield_reason: YieldReason,
    interrupt: Option<InterruptRecord>,
) -> Vec<TraceRecord> {
    let mut rt = build(16, 7, 100);
    rt.set_mode(mode);
    rt.registry_mut().register_with(|id| InterruptingUnit {
        id,
        done: false,
        yield_reason,
        interrupt,
    });
    rt.step().unwrap();
    TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("decode"))
        .filter(|r| matches!(r, TraceRecord::InterruptDelivered { .. }))
        .collect()
}

#[test]
fn a_step_that_ends_at_an_interrupt_boundary_records_the_vector_srr0_and_tick() {
    let want = vec![TraceRecord::InterruptDelivered {
        unit: UnitId::new(0),
        vector: 0xC00,
        srr0: 0x0002_0004,
        time: GuestTicks::new(7),
    }];
    let boundary = YieldReason::InterruptBoundary;
    assert_eq!(
        delivered(RuntimeMode::FullTrace, boundary, Some(RECORD)),
        want
    );
    assert_eq!(
        delivered(RuntimeMode::DeterminismCheck, boundary, Some(RECORD)),
        want
    );
    assert_eq!(
        delivered(RuntimeMode::FaultDriven, boundary, Some(RECORD)),
        Vec::new()
    );
}

#[test]
fn a_step_with_no_delivered_interrupt_records_none() {
    let mode = RuntimeMode::FullTrace;
    assert_eq!(
        delivered(mode, YieldReason::InterruptBoundary, None),
        Vec::new()
    );
    assert_eq!(
        delivered(mode, YieldReason::BudgetExhausted, Some(RECORD)),
        Vec::new(),
        "an interrupt record on another yield is no delivery"
    );
}
