//! An `lwarx` that takes an interrupt on a thread with supervisor
//! registers does not retire. It counts no execution, and the attempt
//! that runs again counts it once.

use crate::supervisor::SupervisorState;
use crate::PpuExecutionUnit;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::ppu_interrupt::VECTOR_ALIGNMENT;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_SF;
use cellgov_time::Budget;

/// Where the code starts, at a real address.
const CODE: u64 = 0x3000;
/// Two bytes past a word: an `lwarx` there takes the alignment interrupt.
const UNALIGNED: u64 = 0x5002;
/// `addi r3,r0,7`.
const LI_R3_7: u32 = (14 << 26) | (3 << 21) | 7;
/// `lwarx r6,0,r7`.
const LWARX_R6_0_R7: u32 = (31 << 26) | (6 << 21) | (7 << 11) | (20 << 1);

/// A real mode thread at [`CODE`] with the words `code` there, and r7 at
/// [`UNALIGNED`].
fn thread(code: &[u32]) -> (PpuExecutionUnit, GuestMemory) {
    let mut mem = GuestMemory::new(0x8_0000);
    for (i, word) in code.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(CODE + 4 * i as u64), 4).expect("a range");
        mem.apply_commit(range, &word.to_be_bytes())
            .expect("the memory holds the code");
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut()
        .set_supervisor(Some(SupervisorState::new(MSR_SF)));
    unit.state_mut().pc = CODE;
    unit.state_mut().set_gpr(7, UNALIGNED);
    (unit, mem)
}

fn run(unit: &mut PpuExecutionUnit, mem: &GuestMemory, budget: u64) -> ExecutionStepResult {
    unit.run_until_yield(
        Budget::new(budget),
        &ExecutionContext::new(mem),
        &mut Vec::new(),
    )
}

#[test]
fn an_lwarx_that_takes_an_interrupt_first_in_its_batch_counts_no_execution_at_any_budget() {
    for budget in [1, 8] {
        let (mut unit, mem) = thread(&[LWARX_R6_0_R7]);
        let r = run(&mut unit, &mem, budget);
        assert_eq!(
            r.yield_reason,
            YieldReason::InterruptBoundary,
            "budget {budget}: {:?}",
            r.local_diagnostics
        );
        assert_eq!(unit.state().pc, VECTOR_ALIGNMENT, "budget {budget}");
        assert_eq!(unit.state().lwarx_executed, 0, "budget {budget}");
    }
}

#[test]
fn an_lwarx_that_faults_behind_a_retired_instruction_counts_no_execution_before_it_runs_again() {
    let (mut unit, mem) = thread(&[LI_R3_7, LWARX_R6_0_R7]);
    let r = run(&mut unit, &mem, 8);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "{:?}",
        r.local_diagnostics
    );
    assert_eq!(
        (unit.retired_instructions(), unit.state().pc),
        (1, CODE + 4)
    );
    assert_eq!(
        unit.state().lwarx_executed,
        0,
        "the lwarx did not retire, and the next batch runs it again"
    );
    let r = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(unit.state().pc, VECTOR_ALIGNMENT);
    assert_eq!(unit.state().lwarx_executed, 0);
}
