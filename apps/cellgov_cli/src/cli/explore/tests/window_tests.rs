//! Where the exploration window opens over a fake-ISA runtime.

use super::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{GuestMemory, MemError};
use cellgov_time::{Budget, InstructionCost};

fn runtime(units: usize, max_steps: usize, ops: &[FakeOp]) -> cellgov_core::Runtime {
    let mut rt = cellgov_core::Runtime::new(GuestMemory::new(64), Budget::new(100), max_steps);
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
            fault: Some(cellgov_effects::FaultKind::Guest(GUEST_FAULT_CODE)),
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

/// The checkpoint every case below runs under; the RSX cases name
/// their own.
const EXITS: CheckpointTrigger = CheckpointTrigger::ProcessExit;

/// The refusal a drive ended with, as the operator reads it.
fn line(e: WindowNeverOpened) -> String {
    e.to_string()
}

#[test]
fn two_runnable_units_open_the_window_before_any_step_retires() {
    let mut rt = runtime(2, 100, &THREE_OPS);
    assert_eq!(
        open_window(&mut rt, WindowStart::FirstBranchingPoint, EXITS),
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
    assert_eq!(open_window(&mut rt, WindowStart::Step(2), EXITS), Ok(2));
    assert_eq!(rt.steps_taken(), 2);
}

#[test]
fn a_single_unit_workload_never_opens_a_branching_point_window() {
    let mut rt = runtime(1, 100, &THREE_OPS);
    assert_eq!(
        open_window(&mut rt, WindowStart::FirstBranchingPoint, EXITS),
        Err(WindowNeverOpened {
            start: WindowStart::FirstBranchingPoint,
            checkpoint: EXITS,
            steps: 3,
            stop: WindowStop::Run(StopReason::Stalled),
        }),
    );
}

#[test]
fn the_step_cap_closes_a_window_that_never_opened_and_names_itself_a_bound() {
    // The single unit needs 3 steps and never branches; the cap refuses
    // the 3rd.
    let mut rt = runtime(1, 2, &THREE_OPS);
    let e = open_window(&mut rt, WindowStart::FirstBranchingPoint, EXITS)
        .expect_err("a single-unit workload has no branching point");
    assert_eq!(e.steps, 2);
    assert_eq!(
        e.stop,
        WindowStop::Run(StopReason::StepError(StepError::MaxStepsExceeded))
    );
    assert_eq!(
        e.stop.label(),
        "bound",
        "the cap is the caller's own and must not read as a model refusal",
    );
    assert!(line(e).contains("(bound)"), "{}", line(e));
}

#[test]
fn a_pc_start_the_workload_never_retires_never_opens() {
    let mut rt = runtime(1, 100, &THREE_OPS);
    let e = open_window(&mut rt, WindowStart::Pc(0xDEAD_BEEF), EXITS)
        .expect_err("the fake ISA retires no guest pc");
    assert_eq!(e.stop, WindowStop::Run(StopReason::Stalled));
    assert!(line(e).contains("pc 0xdeadbeef"), "{}", line(e));
}

#[test]
fn a_start_step_at_or_past_the_step_cap_is_refused_before_the_boot_runs() {
    // The refusal has to name both numbers: an operator who reads only
    // "too large" cannot tell which of the two flags to move.
    let at = start_past_cap(WindowStart::Step(100), 100).expect("the cap is not past itself");
    assert!(at.contains("100"), "{at}");
    assert!(at.contains("--start-step"), "{at}");
    assert!(at.contains("--max-steps"), "{at}");

    let past = start_past_cap(WindowStart::Step(4_096), 100).expect("4096 is past the cap");
    assert!(past.contains("4096"), "{past}");
    assert!(past.contains("100"), "{past}");

    assert!(start_past_cap(WindowStart::Step(99), 100).is_none());
    assert!(
        start_past_cap(WindowStart::FirstBranchingPoint, 1).is_none(),
        "only a step count can be placed past the cap",
    );
    assert!(start_past_cap(WindowStart::Pc(0x1000), 1).is_none());
}

#[test]
fn a_faulting_unit_stops_the_drive_at_the_step_its_batch_was_discarded_on() {
    let mut rt = cellgov_core::Runtime::new(GuestMemory::new(64), Budget::new(100), 100);
    rt.register_unit_with(|id| FaultingUnit { id, pc: 0x1_0040 });
    let e = open_window(&mut rt, WindowStart::FirstBranchingPoint, EXITS)
        .expect_err("one unit never branches");
    assert_eq!(
        e.steps, 0,
        "the faulting step committed nothing, so no step preceded the stop",
    );
    assert_eq!(
        e.stop,
        WindowStop::Fault {
            pc: Some(0x1_0040),
            kind: cellgov_effects::FaultKind::Guest(GUEST_FAULT_CODE),
        },
        "a discarded batch must not be absorbed and spun on to the step cap",
    );
    assert_eq!(e.stop.label(), "fault");
    assert!(line(e).contains("0x00010040"), "{}", line(e));
    assert!(line(e).contains("0x00000700"), "{}", line(e));
}

#[test]
fn a_parked_boot_is_not_reported_as_one_that_ran_itself_out() {
    // Nothing is runnable and nothing is queued to wake the unit. Only
    // `Runtime::step` separates that from a finished boot, so the drive
    // must reach it rather than answer for it.
    let mut rt = cellgov_core::Runtime::new(GuestMemory::new(64), Budget::new(100), 100);
    rt.register_unit_with(|id| ParkedUnit { id });
    let e = open_window(&mut rt, WindowStart::FirstBranchingPoint, EXITS)
        .expect_err("nothing is runnable");
    assert_eq!(
        e.stop,
        WindowStop::Run(StopReason::StepError(StepError::AllBlocked))
    );
    assert_eq!(e.stop.label(), "blocked");
}

#[test]
fn a_boot_that_reached_the_cell_s_checkpoint_is_not_reported_as_a_refusal() {
    let rsx_write = WindowStop::Run(StopReason::CommitError(cellgov_core::CommitError::Memory(
        MemError::ReservedWrite {
            addr: 0x0C00_0000,
            region: "rsx",
        },
    )));
    let at_checkpoint = line(WindowNeverOpened {
        start: WindowStart::Step(1_000),
        checkpoint: CheckpointTrigger::FirstRsxWrite,
        steps: 900,
        stop: rsx_write,
    });
    assert!(at_checkpoint.contains("first-rsx-write"), "{at_checkpoint}");
    assert!(at_checkpoint.contains("0x0c000000"), "{at_checkpoint}");
    assert!(
        !at_checkpoint.contains("(refusal)"),
        "the cell's own stop is not a refusal the model gave: {at_checkpoint}",
    );

    // The same refusal in a cell that stops elsewhere keeps its class.
    let elsewhere = line(WindowNeverOpened {
        start: WindowStart::Step(1_000),
        checkpoint: EXITS,
        steps: 900,
        stop: rsx_write,
    });
    assert!(elsewhere.contains("(refusal)"), "{elsewhere}");
}

/// A real `sys_process_spawn` drives the trigger, and only the
/// `microtests` corpus reaches one; this case pins the line an operator
/// reads when it fires.
#[test]
fn an_unserved_child_init_is_reported_as_neither_a_stall_nor_a_refusal() {
    let e = WindowNeverOpened {
        start: WindowStart::FirstBranchingPoint,
        checkpoint: EXITS,
        steps: 412,
        stop: WindowStop::ChildInitUnserved,
    };
    assert_eq!(e.stop.label(), "unserved");
    let text = line(e);
    assert!(text.contains("412"), "{text}");
    assert!(text.contains("spawned a child"), "{text}");
    assert!(text.contains("(unserved)"), "{text}");
    assert!(
        !text.contains("(finished)") && !text.contains("(refusal)"),
        "a parked child is neither a boot that ran itself out nor a refusal: {text}",
    );
}
