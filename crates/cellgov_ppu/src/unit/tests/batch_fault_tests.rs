//! A mid-batch fault yields the fault site's registers while the unit
//! rewinds to batch entry.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

fn place_insn(mem: &mut GuestMemory, offset: usize, raw: u32) {
    let range = ByteRange::new(GuestAddr::new(offset as u64), 4).unwrap();
    mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
}

/// `addi rT, rT, 1`.
fn addi_inc(rt: u32) -> u32 {
    (14 << 26) | (rt << 21) | (rt << 16) | 1
}

const UNMAPPED_EA: u64 = 0xFFFF_0000;

/// Two increments retire, then `third` runs with r5 aimed at `UNMAPPED_EA`.
fn unit_after_two_retired_then_fault(third: u32) -> (PpuExecutionUnit, ExecutionStepResult) {
    let mut mem = GuestMemory::new(256);
    place_insn(&mut mem, 0, addi_inc(3));
    place_insn(&mut mem, 4, addi_inc(4));
    place_insn(&mut mem, 8, third);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(3, 10);
    unit.state_mut().set_gpr(4, 20);
    unit.state_mut().set_gpr(5, UNMAPPED_EA);
    unit.state_mut().set_lr(0x1000);

    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
    assert!(effects.is_empty(), "leaked effects: {effects:?}");
    (unit, result)
}

#[test]
fn mem_fault_diagnostics_carry_the_fault_site_registers_not_the_entry_snapshot() {
    let lwz_r6_r5: u32 = (32 << 26) | (6 << 21) | (5 << 16);
    let (unit, result) = unit_after_two_retired_then_fault(lwz_r6_r5);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(result.consumed_cost.raw(), 0);
    assert_eq!(result.local_diagnostics.pc, Some(8));
    assert_eq!(result.local_diagnostics.faulting_ea, Some(UNMAPPED_EA));
    let regs = result
        .local_diagnostics
        .fault_regs
        .as_ref()
        .expect("fault carries a register dump");
    assert_eq!(regs.gprs[3], 11, "dump reflects the retired addi r3");
    assert_eq!(regs.gprs[4], 21, "dump reflects the retired addi r4");
    assert_eq!(regs.lr, 0x1000);

    assert_eq!(unit.status(), UnitStatus::Faulted);
    assert_eq!(unit.state().gpr[3], 10, "unit rewinds to batch entry");
    assert_eq!(unit.state().gpr[4], 20, "unit rewinds to batch entry");
    assert_eq!(unit.state().pc, 0);
    assert_eq!(unit.state().mem_fault_arm_entries, 1);
    assert_eq!(unit.state().mem_fault_unmapped_routed, 1);
}

#[test]
fn decode_fault_diagnostics_carry_the_fault_site_registers_not_the_entry_snapshot() {
    // The all-zero word at 8 fails to decode.
    let (unit, result) = unit_after_two_retired_then_fault(0);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(result.consumed_cost.raw(), 0);
    assert_eq!(result.local_diagnostics.pc, Some(8));
    let regs = result
        .local_diagnostics
        .fault_regs
        .as_ref()
        .expect("fault carries a register dump");
    assert_eq!(regs.gprs[3], 11);
    assert_eq!(regs.gprs[4], 21);

    assert_eq!(unit.status(), UnitStatus::Faulted);
    assert_eq!(unit.state().gpr[3], 10);
    assert_eq!(unit.state().gpr[4], 20);
    assert_eq!(unit.state().pc, 0);
}
