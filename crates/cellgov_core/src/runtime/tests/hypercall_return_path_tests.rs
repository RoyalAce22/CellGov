//! The runtime gives the caller the r0, r11, r12 and CR fields of lv1's
//! return for each answer, beside r3 to r10.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    CompareFields, CompareResult, ExecutionContext, ExecutionStepResult, ExecutionUnit,
    LocalDiagnostics, SaveRestore, UnitStatus, YieldReason,
};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::dispatch::{CALLER_TSRL, UNKNOWN_HYPERCALL_R3};
use cellgov_ps3_abi::lv1::partition::LPAR_ID_GAME_OS;
use cellgov_time::{Budget, InstructionCost};

use crate::runtime::types::{RuntimeMode, StepError};
use crate::runtime::Runtime;

/// The address of the scripted `sc`.
const PC: u64 = 0x200;

/// The SRR0 and SRR1 that the caller's `sc` sets.
const SAVE_RESTORE: SaveRestore = SaveRestore {
    srr0: PC + 4,
    srr1: 0x8000_0000_0000_C032,
};

/// What the caller's step after the hypercall received: r3, every
/// register write, and the CR fields.
type Inputs = (Option<u64>, Vec<(u8, u64)>, Option<CompareFields>);

/// Where the caller keeps its [`Inputs`].
type Received = Rc<RefCell<Option<Inputs>>>;

/// A unit that yields one hypercall, then records what its next step
/// receives and finishes.
#[derive(Clone)]
struct Caller {
    id: UnitId,
    args: [u64; 9],
    problem_state: bool,
    save_restore: Option<SaveRestore>,
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
    fn system_call_save_restore(&self) -> Option<SaveRestore> {
        self.save_restore
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
                local_diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(PC, 0, 1),
                fault: None,
                syscall_args: Some(self.args),
            };
        }
        *self.received.borrow_mut() = Some((
            ctx.syscall_return(),
            ctx.register_writes().to_vec(),
            ctx.compare_fields(),
        ));
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

/// Runs hypercall `number` from a caller in the given processor state,
/// on a 4.93 hypervisor, and returns what the caller's next step
/// received.
fn run(number: u64, problem_state: bool, save_restore: Option<SaveRestore>) -> Inputs {
    run_runtime(number, problem_state, save_restore).1
}

/// [`run`], with the runtime after the caller's next step.
fn run_runtime(
    number: u64,
    problem_state: bool,
    save_restore: Option<SaveRestore>,
) -> (Runtime, Inputs) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(4), 100);
    rt.set_mode(RuntimeMode::FullTrace);
    rt.set_hypervisor_firmware("4.93");
    let received: Received = Rc::new(RefCell::new(None));
    let mut args = [0; 9];
    args[0] = number;
    let seen = Rc::clone(&received);
    rt.register_unit_with(move |id| Caller {
        id,
        args,
        problem_state,
        save_restore,
        steps: 0,
        received: seen,
    });
    for _ in 0..2 {
        step(&mut rt).expect("a step");
    }
    let got = received.borrow().clone();
    (rt, got.expect("the caller's second step ran"))
}

fn step(rt: &mut Runtime) -> Result<(), StepError> {
    let mut s = rt.step()?;
    rt.commit_step(&mut s).expect("commit");
    Ok(())
}

fn fields(set: &[(usize, CompareResult)]) -> CompareFields {
    let mut cr = [None; 8];
    for &(field, result) in set {
        cr[field] = Some(result);
    }
    CompareFields(cr)
}

/// The writes of the return through `hrfid`: r4 to r10, then r0, r11 and
/// r12.
fn hrfid_writes(outputs: [u64; 7]) -> Vec<(u8, u64)> {
    let mut writes: Vec<(u8, u64)> = (4u8..).zip(outputs).collect();
    writes.extend([
        (0, CALLER_TSRL),
        (11, SAVE_RESTORE.srr0),
        (12, SAVE_RESTORE.srr1),
    ]);
    writes
}

#[test]
fn a_number_past_the_table_gets_tsrl_in_r0_the_shifted_number_and_the_table_base() {
    let got = run(0x100, false, Some(SAVE_RESTORE));
    let writes = vec![(0, CALLER_TSRL), (11, 0x800), (12, 0x37_2D08)];
    let cr = fields(&[(0, CompareResult::Equal)]);
    assert_eq!(got, (Some(UNKNOWN_HYPERCALL_R3), writes, Some(cr)));
}

#[test]
fn a_privileged_answer_gets_tsrl_in_r0_the_sc_save_restore_pair_and_cr_0_1_and_7() {
    let got = run(74, false, Some(SAVE_RESTORE));
    let outputs = [LPAR_ID_GAME_OS, 0, 0, 0, 0, 0, 0];
    let cr = fields(&[
        (0, CompareResult::Equal),
        (1, CompareResult::Less),
        (7, CompareResult::Greater),
    ]);
    assert_eq!(got, (Some(0), hrfid_writes(outputs), Some(cr)));
}

#[test]
fn a_problem_state_refusal_gets_eq_in_cr1() {
    let (_, writes, cr) = run(74, true, Some(SAVE_RESTORE));
    assert_eq!(writes, hrfid_writes([0; 7]));
    let expected = fields(&[
        (0, CompareResult::Equal),
        (1, CompareResult::Equal),
        (7, CompareResult::Greater),
    ]);
    assert_eq!(cr, Some(expected));
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "gives no SRR0 and SRR1")]
fn a_caller_with_no_save_restore_pair_panics_in_debug() {
    run(74, false, None);
}

#[cfg(not(debug_assertions))]
#[test]
fn a_caller_with_no_save_restore_pair_keeps_r11_and_r12_and_logs_a_break_in_release() {
    let (rt, (_, writes, _)) = run_runtime(74, false, None);
    let mut expected = hrfid_writes([LPAR_ID_GAME_OS, 0, 0, 0, 0, 0, 0]);
    expected.truncate(8);
    assert_eq!(writes, expected, "r4 to r10 and r0 only");
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("dispatch.hypervisor_save_restore_missing"),
        1
    );
}
