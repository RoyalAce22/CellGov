//! The runtime hands lv1's vector answers to the caller as the exact r3
//! lv1 returns, and passes the caller's processor state to the
//! hypervisor.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_lv1::{HypercallRefusal, HypercallWitness};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::dispatch::{PROBLEM_STATE_DENIED_R3, UNKNOWN_HYPERCALL_R3};
use cellgov_ps3_abi::lv1::table::INVALID_HYPERCALL_R3;
use cellgov_time::{Budget, GuestTicks, InstructionCost};
use cellgov_trace::{TraceReader, TraceRecord, TracedHypercallDisposition};

use crate::runtime::types::{RuntimeMode, StepError};
use crate::runtime::Runtime;

/// What the caller's step after the hypercall received: r3, and the
/// register writes.
type Inputs = (Option<u64>, Vec<(u8, u64)>);

/// Where the caller keeps its [`Inputs`].
type Received = Rc<RefCell<Option<Inputs>>>;

/// A unit that yields one hypercall from the processor state it names,
/// then records what its next step receives and finishes.
#[derive(Clone)]
struct Caller {
    id: UnitId,
    args: [u64; 9],
    problem_state: bool,
    steps: u32,
    received: Received,
}

impl ExecutionUnit for Caller {
    type Snapshot = ();
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.steps >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn in_problem_state(&self) -> bool {
        self.problem_state
    }
    fn run_until_yield(
        &mut self,
        _budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps += 1;
        if self.steps == 1 {
            return ExecutionStepResult {
                yield_reason: YieldReason::Syscall,
                consumed_cost: InstructionCost::new(3),
                local_diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(0x100, 0, 1),
                fault: None,
                syscall_args: Some(self.args),
            };
        }
        *self.received.borrow_mut() = Some((ctx.syscall_return(), ctx.register_writes().to_vec()));
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) {}
}

/// A traced runtime for the firmware version `firmware`, whose one
/// caller makes hypercall `number` with r4 to r10 holding 9, and where
/// the caller keeps what its next step receives.
fn runtime_with(firmware: Option<&str>, number: u64, problem_state: bool) -> (Runtime, Received) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 100);
    rt.set_mode(RuntimeMode::FullTrace);
    if let Some(version) = firmware {
        rt.set_hypervisor_firmware(version);
    }
    let received: Received = Rc::new(RefCell::new(None));
    let mut args = [9; 9];
    args[0] = number;
    let seen = Rc::clone(&received);
    rt.register_unit_with(move |id| Caller {
        id,
        args,
        problem_state,
        steps: 0,
        received: seen,
    });
    (rt, received)
}

/// Runs one step and commits it.
fn step(rt: &mut Runtime) -> Result<(), StepError> {
    let mut s = rt.step()?;
    rt.commit_step(&mut s).expect("commit");
    Ok(())
}

/// A 4.93 runtime whose one caller makes hypercall `number` with r4 to
/// r10 holding 9, runs it and the caller's next step, and returns what
/// that step received.
fn run_at_4_93(number: u64, problem_state: bool) -> (Runtime, Inputs) {
    let (mut rt, received) = runtime_with(Some("4.93"), number, problem_state);
    for _ in 0..2 {
        step(&mut rt).expect("a step");
    }
    let got = received
        .borrow()
        .clone()
        .expect("the caller's second step ran");
    (rt, got)
}

fn cleared_r4_to_r10() -> Vec<(u8, u64)> {
    (4..=10).map(|reg| (reg, 0)).collect()
}

#[test]
fn a_problem_state_call_receives_no_privilege_zero_extended_and_cleared_outputs() {
    let (rt, got) = run_at_4_93(74, true);
    assert_eq!(got, (Some(PROBLEM_STATE_DENIED_R3), cleared_r4_to_r10()));
    assert_eq!(got.0, Some(0x0000_0000_FFFF_FFFD));
    assert_eq!(rt.hypervisor().refusal(), None);
}

#[test]
fn an_invalid_number_receives_the_invalid_function_r3_and_keeps_its_witness() {
    let (rt, got) = run_at_4_93(14, false);
    assert_eq!(got, (Some(INVALID_HYPERCALL_R3), cleared_r4_to_r10()));
    assert_eq!(rt.hypervisor().refusal(), None);
    let witness = rt.hypervisor().witnesses().get(&14).copied();
    // The caller's hypercall step costs 3 ticks, and the call follows it.
    assert_eq!(
        witness,
        Some(HypercallWitness {
            hits: 1,
            first_tick: GuestTicks::new(3),
        })
    );
}

#[test]
fn a_number_past_the_table_receives_not_implemented_and_no_output() {
    let (rt, got) = run_at_4_93(256, false);
    assert_eq!(got, (Some(UNKNOWN_HYPERCALL_R3), Vec::new()));
    assert_eq!(rt.hypervisor().refusal(), None);
}

#[test]
fn a_problem_state_call_with_no_known_table_is_traced_as_refused_and_stops_the_run() {
    let (mut rt, received) = runtime_with(None, 74, true);
    step(&mut rt).expect("the hypercall step");
    assert_eq!(step(&mut rt), Err(StepError::HypercallRefused));
    assert_eq!(*received.borrow(), None, "the caller never ran again");
    let unit = UnitId::new(0);
    assert_eq!(
        rt.hypervisor().refusal(),
        Some(HypercallRefusal::ProblemState {
            number: 74,
            unit,
            pc: 0x100
        })
    );
    let entered: Vec<TraceRecord> = TraceReader::new(rt.trace().bytes())
        .map(|r| r.expect("decode"))
        .filter(|r| matches!(r, TraceRecord::HypercallEntered { .. }))
        .collect();
    assert_eq!(
        entered,
        vec![TraceRecord::HypercallEntered {
            unit,
            number: 74,
            args: [9; 8],
            pc: 0x100,
            thread: None,
            disposition: TracedHypercallDisposition::Refused,
            time: GuestTicks::new(3),
        }]
    );
}
