//! A fused pair retires both of its slots or runs as its first
//! instruction alone, so its second slot runs exactly once.

use crate::*;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

use crate::instruction::PpuInstruction;

const LI_R3_7: u32 = (14 << 26) | (3 << 21) | 7;
/// `stw r3, 0x80(r4)`.
const STW_R3_R4: u32 = (36 << 26) | (3 << 21) | (4 << 16) | 0x80;

/// A unit over `li r3,7; stw r3,0x80(r4)` with the pair fused.
fn fused_li_stw() -> (GuestMemory, PpuExecutionUnit) {
    let mut mem = GuestMemory::new(0x100);
    let mut code = [0u8; 8];
    code[..4].copy_from_slice(&LI_R3_7.to_be_bytes());
    code[4..].copy_from_slice(&STW_R3_R4.to_be_bytes());
    mem.apply_commit(ByteRange::new(GuestAddr::new(0), 8).unwrap(), &code)
        .unwrap();
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert_eq!(
        shadow.get(4),
        Some(PpuInstruction::Consumed),
        "precondition: the pair fuses"
    );
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    (mem, unit)
}

fn writes(effects: &[Effect]) -> Vec<&Effect> {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::SharedWriteIntent { .. }))
        .collect()
}

#[test]
fn a_batch_that_ends_at_a_fused_head_runs_the_second_slot_once_in_the_next() {
    let (mem, mut unit) = fused_li_stw();
    let ctx = ExecutionContext::new(&mem);

    let mut first = Vec::new();
    let r = unit.run_until_yield(Budget::new(1), &ctx, &mut first);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, 4, "the head ran as its first instruction");
    assert_eq!(unit.state().gpr[3], 7);
    assert!(writes(&first).is_empty(), "the store has not run yet");

    let mut second = Vec::new();
    let r = unit.run_until_yield(Budget::new(1), &ctx, &mut second);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, 8);
    assert_eq!(writes(&second).len(), 1, "the store runs once");
    assert_eq!(unit.retired_instructions(), 2);
}

#[test]
fn a_fused_pair_retires_both_slots_when_the_budget_holds_two() {
    let (mem, mut unit) = fused_li_stw();
    let ctx = ExecutionContext::new(&mem);

    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(2), &ctx, &mut effects);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, 8);
    assert_eq!(writes(&effects).len(), 1);
    assert_eq!(unit.retired_instructions(), 2);
}

#[test]
fn a_break_on_the_second_slot_stops_before_it_runs() {
    let (mem, mut unit) = fused_li_stw();
    unit.set_break_pc(4, 0);
    let ctx = ExecutionContext::new(&mem);

    let r = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, 4);
    let r = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(r.fault, Some(FaultKind::Guest(FAULT_DEBUG_BREAK)));
    assert_eq!(r.local_diagnostics.pc, Some(4));
    let regs = r
        .local_diagnostics
        .fault_regs
        .expect("a break dumps registers");
    assert_eq!(
        regs.gprs[3], 7,
        "the first instruction ran before the break"
    );
}
