//! Where a window opens over a fake-ISA runtime, and what ends a drive
//! that never gets there.

use super::*;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_time::{Budget, InstructionCost};

fn runtime(units: usize, max_steps: usize, ops: &[FakeOp]) -> Runtime {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(100), max_steps);
    for _ in 0..units {
        rt.register_unit_with(|id| FakeIsaUnit::new(id, ops.to_vec()));
    }
    rt
}

const THREE_OPS: [FakeOp; 3] = [
    FakeOp::LoadImm(0xAA),
    FakeOp::SharedStore { addr: 0, len: 4 },
    FakeOp::End,
];

const GUEST_FAULT_CODE: u32 = 0x0000_0700;

/// A unit that yields a discarded batch every time it runs, and stays
/// runnable so a driver that absorbs the fault spins to the step cap.
#[derive(Clone)]
struct FaultingUnit {
    id: UnitId,
    pc: u64,
}

impl ExecutionUnit for FaultingUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
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
            yield_reason: YieldReason::Fault,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::with_pc(self.pc),
            fault: Some(FaultKind::Guest(GUEST_FAULT_CODE)),
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

/// A unit parked with nothing queued to wake it.
#[derive(Clone)]
struct ParkedUnit {
    id: UnitId,
}

impl ExecutionUnit for ParkedUnit {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Blocked
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
fn two_runnable_units_open_the_window_before_any_step_retires() {
    let mut rt = runtime(2, 100, &THREE_OPS);
    assert_eq!(
        open_window(&mut rt, WindowStart::FirstBranchingPoint),
        Ok(0)
    );
    assert_eq!(
        rt.steps_taken(),
        0,
        "the window must cover the branching point itself, not the step after it",
    );
}

#[test]
fn a_step_start_takes_exactly_that_many_runtime_steps_first() {
    let mut rt = runtime(1, 100, &THREE_OPS);
    assert_eq!(open_window(&mut rt, WindowStart::Step(2)), Ok(2));
    assert_eq!(rt.steps_taken(), 2);
}

#[test]
fn a_single_unit_workload_never_opens_a_branching_point_window() {
    let mut rt = runtime(1, 100, &THREE_OPS);
    assert_eq!(
        open_window(&mut rt, WindowStart::FirstBranchingPoint),
        Err(WindowNeverOpened {
            start: WindowStart::FirstBranchingPoint,
            steps: 3,
            stop: DrivenStop {
                reason: StopReason::Stalled,
                pc: None,
            },
        }),
    );
}

#[test]
fn the_step_cap_closes_a_window_that_never_opened_and_names_itself_a_bound() {
    // The single unit needs 3 steps and never branches; the cap refuses
    // the 3rd.
    let mut rt = runtime(1, 2, &THREE_OPS);
    let e = open_window(&mut rt, WindowStart::FirstBranchingPoint)
        .expect_err("a single-unit workload has no branching point");
    assert_eq!(e.steps, 2);
    assert_eq!(
        e.stop.reason,
        StopReason::StepError(StepError::MaxStepsExceeded)
    );
    assert_eq!(
        e.stop.reason.class(),
        StopClass::Bound,
        "the cap is the caller's own and must not read as a model refusal",
    );
    let line = e.to_string();
    assert!(line.contains("first branching point"), "{line}");
    assert!(line.contains("after 2 step(s)"), "{line}");
    assert!(line.ends_with("(bound)"), "{line}");
}

#[test]
fn a_pc_start_the_workload_never_retires_never_opens() {
    let mut rt = runtime(1, 100, &THREE_OPS);
    let e = open_window(&mut rt, WindowStart::Pc(0xDEAD_BEEF))
        .expect_err("the fake ISA retires no guest pc");
    assert_eq!(e.stop.reason, StopReason::Stalled);
    assert_eq!(e.start.to_string(), "pc 0xdeadbeef");
}

#[test]
fn a_faulting_unit_stops_the_drive_at_the_step_its_batch_was_discarded_on() {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(100), 100);
    rt.register_unit_with(|id| FaultingUnit { id, pc: 0x1_0040 });
    let e = open_window(&mut rt, WindowStart::FirstBranchingPoint)
        .expect_err("one unit never branches");
    assert_eq!(
        e.steps, 0,
        "the faulting step committed nothing, so no step preceded the stop",
    );
    assert_eq!(
        rt.steps_taken(),
        1,
        "the runtime counts the faulting step the report stops short of",
    );
    assert_eq!(
        e.stop,
        DrivenStop {
            reason: StopReason::Faulted(FaultKind::Guest(GUEST_FAULT_CODE)),
            pc: Some(0x1_0040),
        },
        "a discarded batch must not be absorbed and spun on to the step cap",
    );
}

/// Nothing is runnable and nothing is queued to wake the unit. The
/// drive reports the deadlock [`run_to_stall`] reports, not a refused
/// step.
#[test]
fn a_parked_boot_is_a_deadlock_as_run_to_stall_reads_it() {
    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(100), 100);
    rt.register_unit_with(|id| ParkedUnit { id });
    let e =
        open_window(&mut rt, WindowStart::FirstBranchingPoint).expect_err("nothing is runnable");
    assert_eq!(e.stop.reason, StopReason::Deadlocked);

    let mut rt = Runtime::new(GuestMemory::new(64), Budget::new(100), 100);
    rt.register_unit_with(|id| ParkedUnit { id });
    assert_eq!(run_to_stall(&mut rt, 100), e.stop.reason);
}
