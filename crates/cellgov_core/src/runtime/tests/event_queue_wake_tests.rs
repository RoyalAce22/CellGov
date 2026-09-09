use super::*;
use cellgov_effects::Effect;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, YieldReason,
};
use cellgov_lv2::EventPayload;
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

#[derive(Clone)]
struct IdleUnit(UnitId);

impl ExecutionUnit for IdleUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.0
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Runnable
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: YieldReason::BudgetExhausted,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

#[test]
fn an_event_queue_receive_wake_stages_the_event_in_r4_to_r7_beside_r3_zero() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 100);
    let waiter = rt.registry_mut().register_with(IdleUnit);
    rt.registry_mut()
        .set_status_override(waiter, UnitStatus::Blocked);
    let _ = rt.syscall_responses_mut().insert(
        waiter,
        PendingResponse::EventQueueReceive {
            out_ptr: 0x100,
            payload: Some(EventPayload {
                source: 0x11,
                data1: 0x22,
                data2: 0x33,
                data3: 0x44,
            }),
        },
    );
    rt.resolve_sync_wakes(&[waiter]);
    // `step()` applies register writes only alongside a syscall
    // return, so both halves must be staged together.
    assert_eq!(rt.registry_mut().drain_syscall_return(waiter), Some(0));
    assert_eq!(
        rt.registry_mut().drain_register_writes(waiter),
        vec![(4, 0x11), (5, 0x22), (6, 0x33), (7, 0x44)]
    );
    assert_eq!(
        rt.registry_mut().effective_status(waiter),
        Some(UnitStatus::Runnable)
    );
}
