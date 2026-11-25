//! A traced batch that ends at interrupt entry records the entry's
//! context synchronization.

use super::interrupt_support::*;
use cellgov_exec::{CodeEvent, ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::MSR_EE;
use cellgov_time::Budget;

/// Runs one batch of at most `budget` instructions, traced when `traced`,
/// and returns how it yielded and the code events it left.
fn run_traced(
    unit: &mut crate::PpuExecutionUnit,
    mem: &GuestMemory,
    budget: u64,
    traced: bool,
) -> (YieldReason, u64, Vec<CodeEvent>) {
    let ctx = ExecutionContext::new(mem).with_trace_per_step(traced);
    let r = unit.run_until_yield(Budget::new(budget), &ctx, &mut Vec::new());
    (r.yield_reason, unit.state().pc, unit.drain_code_events())
}

/// [PPC-Book3 p:61 s:5.2] Every interrupt except system reset and machine check is context synchronizing.
#[test]
fn an_interrupt_an_instruction_causes_records_a_context_synchronization() {
    let mem = memory(&[ILLEGAL]);
    for budget in [1, 8] {
        let mut unit = kernel_unit(KERNEL_MSR);
        let (yielded, pc, events) = run_traced(&mut unit, &mem, budget, true);
        assert_eq!(yielded, YieldReason::InterruptBoundary, "budget {budget}");
        assert_eq!(pc, irq::VECTOR_PROGRAM, "budget {budget}");
        assert_eq!(events, [CodeEvent::ContextSync], "budget {budget}");
    }
}

/// [PPC-Book3 p:61 s:5.2] Every interrupt except system reset and machine check is context synchronizing.
#[test]
fn an_external_interrupt_between_two_instructions_records_a_context_synchronization() {
    let mem = memory(&[NOP, NOP]);
    let mut unit = kernel_unit(KERNEL_MSR | MSR_EE);
    assert!(unit.raise_external_exception());
    let (yielded, pc, events) = run_traced(&mut unit, &mem, 8, true);
    assert_eq!(yielded, YieldReason::InterruptBoundary);
    assert_eq!(pc, irq::VECTOR_EXTERNAL);
    assert_eq!(events, [CodeEvent::ContextSync]);
}

/// [PPC-Book3 p:4 s:1.6.1] `sc` is context synchronizing.
/// [PPC-Book3 p:61 s:5.2] The system call interrupt it causes is context synchronizing.
#[test]
fn a_system_call_interrupt_records_one_context_synchronization_after_the_fetch_of_its_sc() {
    let mem = memory(&[SC]);
    for budget in [1, 8] {
        let mut unit = kernel_unit(KERNEL_MSR);
        let (yielded, pc, events) = run_traced(&mut unit, &mem, budget, true);
        assert_eq!(yielded, YieldReason::InterruptBoundary, "budget {budget}");
        assert_eq!(pc, irq::VECTOR_SYSTEM_CALL, "budget {budget}");
        assert_eq!(
            events,
            [CodeEvent::Fetch { pc: CODE }, CodeEvent::ContextSync],
            "budget {budget}"
        );
    }
}

#[test]
fn an_untraced_batch_records_no_context_synchronization_at_interrupt_entry() {
    let mem = memory(&[ILLEGAL]);
    let mut unit = kernel_unit(KERNEL_MSR);
    let (yielded, _, events) = run_traced(&mut unit, &mem, 8, false);
    assert_eq!(yielded, YieldReason::InterruptBoundary);
    assert!(events.is_empty(), "{events:?}");

    let mut unit = kernel_unit(KERNEL_MSR | MSR_EE);
    assert!(unit.raise_external_exception());
    let (yielded, _, events) = run_traced(&mut unit, &mem, 8, false);
    assert_eq!(yielded, YieldReason::InterruptBoundary);
    assert!(events.is_empty(), "{events:?}");
}
