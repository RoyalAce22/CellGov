use super::interrupt_support::*;
use crate::PpuExecutionUnit;
use cellgov_exec::{
    CodeEvent, ExecutionContext, ExecutionStepResult, ExecutionUnit, InterruptRecord, YieldReason,
};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_BE, MSR_EE, MSR_FP, MSR_ME, MSR_RI, MSR_SE, MSR_SF};
use cellgov_time::Budget;

/// `b .+8`.
const B_PLUS_8: u32 = 0x4800_0008;
/// `beq .+8`, which CR0 = 0 does not take.
const BEQ_PLUS_8: u32 = 0x4182_0008;
/// `fadd f1,f2,f3`.
const FADD: u32 = 0xFC22_182A;

/// The trace record with SRR0 `srr0` for a thread whose MSR was `msr`.
///
/// [CBE-Handbook p:257 s:9.5.14] Table 9-16: a trace interrupt sets SRR1 bit 43, clears bits 33:36, 42 and 44:47, and copies the MSR into the rest.
fn trace(srr0: u64, msr: u64) -> InterruptRecord {
    InterruptRecord {
        vector: irq::VECTOR_TRACE,
        srr0,
        srr1: msr | irq::SRR1_TRACE,
        dsisr: None,
        dar: None,
    }
}

/// Returns the first of up to 4 batches of 4 instructions that yields at an interrupt boundary.
fn run_to_interrupt(unit: &mut PpuExecutionUnit, code: &[u32]) -> ExecutionStepResult {
    let mem = memory(code);
    for _ in 0..4 {
        let (r, _) = run(unit, &mem, 4);
        if r.yield_reason == YieldReason::InterruptBoundary {
            return r;
        }
    }
    panic!("no interrupt within 4 batches");
}

/// [PPC-Book3 p:73 s:5.5.14] With MSR\[SE\] = 1 a successfully completed instruction causes a trace interrupt, with SRR0 at the next instruction.
#[test]
fn single_step_traces_the_first_instruction_and_commits_it() {
    let msr = KERNEL_MSR | MSR_SE;
    let mut unit = kernel_unit(msr);
    let r = run_to_interrupt(&mut unit, &[addi(3, 0, 1), addi(3, 0, 2)]);
    assert_eq!(r.local_diagnostics.interrupt, Some(trace(CODE + 4, msr)));
    assert_eq!(r.local_diagnostics.pc, Some(CODE));
    assert_eq!(unit.state().gpr[3], 1);
    assert_eq!(unit.state().pc, irq::VECTOR_TRACE);
    let s = sup(&unit);
    assert_eq!((s.srr0, s.srr1), (CODE + 4, msr | irq::SRR1_TRACE));
    // [CBE-Handbook p:292 s:9.12] Table 9-40: entry clears SE, BE, FP and RI, sets SF and keeps ME.
    assert_eq!(s.msr, MSR_SF | MSR_ME);
}

/// [PPC-Book3 p:10 s:2.2.3] With MSR\[BE\] = 1 the processor takes a trace interrupt after each branch, whether or not the branch is taken.
#[test]
fn branch_trace_traces_each_branch_taken_or_not_and_nothing_else() {
    let msr = KERNEL_MSR | MSR_BE;
    for (code, branch_pc, srr0) in [
        (vec![NOP, B_PLUS_8, NOP, NOP], CODE + 4, CODE + 12),
        (vec![NOP, BEQ_PLUS_8, NOP, NOP], CODE + 4, CODE + 8),
    ] {
        let mut unit = kernel_unit(msr);
        let r = run_to_interrupt(&mut unit, &code);
        assert_eq!(r.local_diagnostics.interrupt, Some(trace(srr0, msr)));
        assert_eq!(r.local_diagnostics.pc, Some(branch_pc));
    }
}

/// [PPC-Book3 p:73 s:5.5.14] With MSR\[SE\] = 0 only a completed branch causes a trace interrupt, and only when MSR\[BE\] = 1.
#[test]
fn branch_trace_alone_runs_a_straight_line_without_an_interrupt() {
    let mem = memory(&[NOP, addi(3, 0, 1), NOP, NOP]);
    let mut unit = kernel_unit(KERNEL_MSR | MSR_BE);
    let (r, _) = run(&mut unit, &mem, 4);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, CODE + 16);
}

/// [PPC-Book3 p:73 s:5.5.14] With MSR\[SE\] = 0 only a completed branch causes a trace interrupt, and only when MSR\[BE\] = 1.
#[test]
fn a_branch_with_se_and_be_clear_runs_without_an_interrupt() {
    let mem = memory(&[B_PLUS_8, NOP, addi(3, 0, 1), NOP]);
    let mut unit = kernel_unit(KERNEL_MSR);
    let (r, _) = run(&mut unit, &mem, 2);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(r.local_diagnostics.interrupt, None);
    assert_eq!(unit.state().gpr[3], 1);
    assert_eq!(unit.state().pc, CODE + 12);
}

/// [CBE-Handbook p:257 s:9.5.14] An mtmsrd that changes MSR\[SE\] from 0 to 1 causes a trace interrupt with SRR0 at the mtmsrd; with SE already 1, SRR0 is the next instruction.
#[test]
fn an_mtmsrd_traces_at_itself_when_it_sets_se_and_at_the_next_instruction_when_se_was_set() {
    for (msr, srr0) in [(KERNEL_MSR, CODE), (KERNEL_MSR | MSR_SE, CODE + 4)] {
        let mut unit = kernel_unit(msr);
        unit.state_mut().set_gpr(5, KERNEL_MSR | MSR_SE);
        let r = run_to_interrupt(&mut unit, &[mtmsrd(5, false), NOP]);
        assert_eq!(
            r.local_diagnostics.interrupt,
            Some(trace(srr0, KERNEL_MSR | MSR_SE))
        );
    }
}

/// [CBE-Handbook p:257 s:9.5.14] An mtmsrd that changes MSR\[SE\] from 1 to 0 causes no trace interrupt.
#[test]
fn an_mtmsrd_that_clears_se_does_not_trace() {
    let mem = memory(&[mtmsrd(5, false), NOP]);
    let mut unit = kernel_unit(KERNEL_MSR | MSR_SE);
    unit.state_mut().set_gpr(5, KERNEL_MSR);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(r.local_diagnostics.interrupt, None);
    assert_eq!(unit.state().pc, CODE + 4);
    assert_eq!(sup(&unit).msr, KERNEL_MSR);
}

/// [PPC-Book3 p:73 s:5.5.14] rfid is never traced; the instruction it returns to is.
#[test]
fn rfid_is_not_traced_and_the_instruction_it_returns_to_is() {
    let msr = KERNEL_MSR | MSR_SE;
    let mut unit = kernel_unit(msr);
    unit.state_mut()
        .update_supervisor(|s| {
            s.srr0 = CODE + 8;
            s.srr1 = msr;
        })
        .expect("supervisor registers");
    let r = run_to_interrupt(&mut unit, &[RFID, NOP, addi(3, 0, 5), NOP]);
    assert_eq!(r.local_diagnostics.interrupt, Some(trace(CODE + 12, msr)));
    assert_eq!(r.local_diagnostics.pc, Some(CODE + 8));
    assert_eq!(unit.state().gpr[3], 5);
}

/// [PPC-Book3 p:73 s:5.5.14] An instruction that causes another interrupt, a trap that traps included, is not traced.
#[test]
fn an_instruction_that_takes_another_interrupt_is_not_traced() {
    for (insn, msr, vector) in [
        (TRAP, KERNEL_MSR | MSR_SE, irq::VECTOR_PROGRAM),
        (
            FADD,
            (KERNEL_MSR & !MSR_FP) | MSR_SE,
            irq::VECTOR_FP_UNAVAILABLE,
        ),
    ] {
        let mut unit = kernel_unit(msr);
        let r = run_to_interrupt(&mut unit, &[insn, NOP]);
        let record = r.local_diagnostics.interrupt.expect("an interrupt record");
        assert_eq!((record.vector, record.srr0), (vector, CODE), "{insn:08x}");
    }
}

/// [CBE-Handbook p:291 s:9.10] Table 9-39: the trace interrupt ranks above the external interrupt, and trace entry clears EE.
#[test]
fn the_trace_comes_before_an_external_interrupt_that_its_instruction_enables() {
    let msr = KERNEL_MSR | MSR_SE;
    let mut unit = kernel_unit(msr);
    assert!(unit.raise_external_exception());
    unit.state_mut().set_gpr(5, MSR_EE);
    let r = run_to_interrupt(&mut unit, &[mtmsrd(5, true), NOP]);
    // mtmsrd with L = 1 takes EE and RI from r5.
    let moved = (msr | MSR_EE) & !MSR_RI;
    assert_eq!(r.local_diagnostics.interrupt, Some(trace(CODE + 4, moved)));
    assert_eq!(sup(&unit).msr & MSR_EE, 0);
    // The external exception still waits for the handler to set EE.
    assert!(sup(&unit).external);
}

/// [PPC-Book3 p:61 s:5.2] Every interrupt except system reset and machine check is context synchronizing.
#[test]
fn a_traced_batch_records_the_trace_entry_context_synchronization_after_the_traced_fetch() {
    let mem = memory(&[addi(3, 0, 1), NOP]);
    let mut unit = kernel_unit(KERNEL_MSR | MSR_SE);
    let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
    let r = unit.run_until_yield(Budget::new(4), &ctx, &mut Vec::new());
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(unit.state().pc, irq::VECTOR_TRACE);
    assert_eq!(
        unit.drain_code_events(),
        [CodeEvent::Fetch { pc: CODE }, CodeEvent::ContextSync]
    );
}
