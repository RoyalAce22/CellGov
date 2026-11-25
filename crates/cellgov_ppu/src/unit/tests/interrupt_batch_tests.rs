//! Interrupt delivery against the batch:
//! - the instructions ahead of a fault commit;
//! - the instruction that faults leaves nothing;
//! - the reservation stays;
//! - a machine check, a checkstop and a CellGov stop end the thread;
//! - `sc` with LEV = 1 yields a hypercall from either processor state,
//!   and ends a thread with no supervisor registers.

use super::interrupt_support::*;
use crate::exec::{PpuFault, PpuInterrupt};
use crate::fault_codes::{FAULT_CHECKSTOP, FAULT_MACHINE_CHECK, FAULT_SYSTEM_CALL};
use crate::PpuExecutionUnit;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_ps3_abi::hw::ppu_interrupt as irq;
use cellgov_ps3_abi::hw::ppu_supervisor::{MSR_DR, MSR_ME, MSR_PR};
use cellgov_sync::{ReservationTable, ReservedLine};
use cellgov_time::Budget;

/// The writes `effects` carries, as (address, bytes).
fn writes(effects: &[Effect]) -> Vec<(u64, Vec<u8>)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, bytes, .. } => {
                Some((range.start().raw(), bytes.bytes().to_vec()))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn the_instructions_ahead_of_a_fault_commit_and_the_faulting_one_leaves_nothing() {
    // li r3,7; stw r3,0(r6); lwarx r4,0,r5 with r5 unaligned.
    let mem = memory(&[addi(3, 0, 7), stw(3, 0, 6), lwarx(4, 0, 5)]);
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().set_gpr(4, 0xAAAA);
    unit.state_mut().set_gpr(5, DATA + 2);
    unit.state_mut().set_gpr(6, DATA);

    let (r, effects) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.retired_instructions(), 2);
    assert_eq!(writes(&effects), vec![(DATA, 7u32.to_be_bytes().to_vec())]);
    assert_eq!(unit.state().pc, CODE + 8);

    let (r, effects) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    let record = r.local_diagnostics.interrupt.expect("an interrupt record");
    assert_eq!(
        (record.vector, record.srr0),
        (irq::VECTOR_ALIGNMENT, CODE + 8)
    );
    assert!(effects.is_empty(), "the faulting lwarx left {effects:?}");
    assert_eq!(unit.retired_instructions(), 2, "the lwarx did not retire");
    assert_eq!(unit.state().gpr[4], 0xAAAA);
    assert_eq!(unit.state().reservation(), None);
}

#[test]
fn interrupt_delivery_puts_back_the_batch_entry_state_and_drops_the_effects() {
    use crate::unit::batch::BatchEntry;
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().set_gpr(4, 0xAAAA);
    let entry = BatchEntry {
        snapshot: Some(unit.state().clone()),
        hashes: 0,
        fulls: 0,
        barriers: 0,
        code_events: 0,
        retired: 0,
        cycle: unit.timing.now(),
        timing: Some(unit.timing.core()),
        trace: false,
    };
    // What a faulting instruction might leave: a register and a read.
    unit.state_mut().set_gpr(4, 0xBBBB);
    let read = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(DATA), 4).expect("a range");
    let mut effects = vec![Effect::SharedReadIntent {
        range: read,
        source: UnitId::new(0),
    }];
    let r = unit.interrupt_yield(
        &entry,
        &mut effects,
        CODE,
        &PpuInterrupt::Program(crate::exec::ProgramCause::Trap { to: 31 }),
    );
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(unit.state().gpr[4], 0xAAAA);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(unit.status(), UnitStatus::Runnable);
    assert_eq!(unit.state().pc, irq::VECTOR_PROGRAM);
}

#[test]
fn a_break_skip_spent_on_a_delivered_fault_is_handed_back() {
    let mem = memory(&[ILLEGAL]);
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.set_break_pc(CODE, 1);
    let (r, _) = run(&mut unit, &mem, 8);
    assert_eq!(r.yield_reason, YieldReason::InterruptBoundary);
    assert_eq!(
        unit.break_skip, 1,
        "the illegal word did not retire, so it was no hit"
    );
}

/// [PPC-Book3 p:63 s:5.4] Interrupts do not clear reservations.
#[test]
fn a_reservation_survives_interrupt_delivery() {
    let mem = memory(&[ILLEGAL]);
    let line = ReservedLine::containing(DATA);
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().set_reservation(Some(line));
    let mut table = ReservationTable::new();
    table.insert_or_replace(UnitId::new(0), line);
    let ctx = ExecutionContext::new(&mem).with_reservations(&table);
    for budget in [1, 8] {
        let mut unit = unit.clone();
        let r = unit.run_until_yield(Budget::new(budget), &ctx, &mut Vec::new());
        assert_eq!(r.yield_reason, YieldReason::InterruptBoundary, "{budget}");
        assert_eq!(unit.state().pc, irq::VECTOR_PROGRAM, "{budget}");
        assert_eq!(unit.state().reservation(), Some(line), "budget {budget}");
    }
}

/// The guest fault code a terminal stop carries.
fn fault_code(kind: Option<FaultKind>) -> Option<u32> {
    match kind {
        Some(FaultKind::Guest(code)) => Some(code),
        _ => None,
    }
}

#[test]
fn a_data_storage_interrupt_is_delivered_and_a_machine_check_sc_1_a_checkstop_and_cellgov_stops_are_not(
) {
    let unit = kernel_unit(KERNEL_MSR);
    let dsi = PpuInterrupt::DataStorage {
        ea: 0,
        store: false,
        protection: false,
    };
    assert_eq!(
        unit.delivered_interrupt(&PpuFault::Interrupt(dsi)),
        Some(dsi)
    );
    for stop in [
        PpuFault::Interrupt(PpuInterrupt::MachineCheck),
        PpuFault::Interrupt(PpuInterrupt::SystemCall { lev: 1 }),
        PpuFault::Checkstop,
        PpuFault::Unmodelled("a state"),
        PpuFault::Internal("an invariant"),
        PpuFault::DebugBreak,
    ] {
        assert_eq!(unit.delivered_interrupt(&stop), None, "{stop:?}");
    }
}

/// [CBE-Handbook p:247 s:9.5.2] A caching-inhibited load from a nonexistent address takes the machine check, and [CBE-Handbook p:293 s:9.15] a checkstop is not an interrupt.
#[test]
fn a_machine_check_and_a_checkstop_stay_terminal_on_a_thread_with_supervisor_registers() {
    const PTE_I: u64 = 0x20;
    const UNBACKED: u64 = 0x50_0000;
    let mut mem = memory(&[lwz(4, 0, 5)]);
    map_page(&mut mem, 4, UNBACKED, 0b10 | PTE_I);
    let mut unit = kernel_unit(KERNEL_MSR | MSR_DR | MSR_ME);
    unit.state_mut().set_gpr(5, SEGMENT + 0x4000);
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(fault_code(r.fault), Some(FAULT_MACHINE_CHECK));
    assert_eq!(unit.status(), UnitStatus::Faulted);

    let mem = memory(&[]);
    let mut unit = kernel_unit(KERNEL_MSR);
    unit.state_mut().pc = 0x8000_0000_0000_0000 | UNBACKED;
    let (r, _) = run(&mut unit, &mem, 1);
    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(fault_code(r.fault), Some(FAULT_CHECKSTOP));
    assert_eq!(unit.status(), UnitStatus::Faulted);
}

/// [PPC-Book3 p:12 s:2.3.1] `sc` with LEV = 1 from problem state invokes the hypervisor, and the architecture calls that a programming error.
///
/// lv1's own vector checks `SRR1[PR]` and answers the call, so the
/// thread yields it to CellGov's hypervisor.
#[test]
fn sc_with_lev_1_in_problem_state_yields_a_hypercall_on_a_thread_with_supervisor_registers() {
    let mem = memory(&[SC_1]);
    let mut unit = kernel_unit(KERNEL_MSR | MSR_PR);
    let (r, _) = run(&mut unit, &mem, 4);
    assert_eq!(r.yield_reason, YieldReason::Syscall);
    assert_eq!(r.fault, None);
    assert_eq!(r.local_diagnostics.syscall_lev, Some(1));
    assert!(unit.in_problem_state());
    assert_eq!(unit.state().pc, CODE);
}

#[test]
fn sc_with_lev_1_in_supervisor_state_yields_a_hypercall_and_reports_no_problem_state() {
    let mem = memory(&[SC_1]);
    let mut unit = kernel_unit(KERNEL_MSR);
    let (r, _) = run(&mut unit, &mem, 4);
    assert_eq!(r.yield_reason, YieldReason::Syscall);
    assert_eq!(r.local_diagnostics.syscall_lev, Some(1));
    assert!(!unit.in_problem_state());
}

#[test]
fn sc_with_lev_1_stays_terminal_on_a_thread_without_supervisor_registers() {
    let mut mem = memory(&[SC_1]);
    put_words(&mut mem, 0, &[SC_1]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let (r, _) = run(&mut unit, &mem, 4);
    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(fault_code(r.fault), Some(FAULT_SYSTEM_CALL | 1));
    assert_eq!(unit.status(), UnitStatus::Faulted);
    assert!(!unit.in_problem_state());
}

#[test]
fn a_thread_without_supervisor_registers_keeps_the_terminal_fault_and_the_syscall_yield() {
    let mut mem = memory(&[ILLEGAL, SC]);
    put_words(&mut mem, 0, &[ILLEGAL, SC]);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let (r, _) = run(&mut unit, &mem, 4);
    assert_eq!(r.yield_reason, YieldReason::Fault);
    let record = r.local_diagnostics.interrupt.expect("an interrupt record");
    assert_eq!(
        (record.vector, record.srr0, record.srr1),
        (irq::VECTOR_PROGRAM, 0, irq::SRR1_PROGRAM_ILLEGAL),
        "SRR1 holds the cause bits only"
    );
    assert_eq!(unit.status(), UnitStatus::Faulted);
    assert_eq!(unit.state().pc, 0);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = 4;
    let (r, _) = run(&mut unit, &mem, 4);
    assert_eq!(r.yield_reason, YieldReason::Syscall);
    assert_eq!(r.local_diagnostics.interrupt, None);
}
