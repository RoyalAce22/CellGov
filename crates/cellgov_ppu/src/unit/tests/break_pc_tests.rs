//! `set_break_pc` accounting across the block-boundary yields.

use crate::*;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, InstructionCost};

const STORE_BUFFER_CAPACITY: usize = 64;

fn place_insn(mem: &mut GuestMemory, offset: usize, raw: u32) {
    let range = ByteRange::new(GuestAddr::new(offset as u64), 4).unwrap();
    mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
}

/// `stw r3, off(r4)`.
fn stw_r3_off_r4(off: u16) -> u32 {
    (36 << 26) | (3 << 21) | (4 << 16) | u32::from(off)
}

/// Stores the block published, apart from the read intents a block
/// also emits for the text it fetched.
fn stores(effects: &[cellgov_effects::Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, cellgov_effects::Effect::SharedWriteIntent { .. }))
        .count()
}

#[test]
fn a_break_skip_is_not_spent_by_a_store_the_full_buffer_retries() {
    const BREAK_PC: u64 = (STORE_BUFFER_CAPACITY * 4) as u64;
    let mut mem = GuestMemory::new(0x1000);
    // 64 stores fill the buffer; the 65th, at BREAK_PC, yields
    // BufferFull and is presented again at the next block.
    for i in 0..STORE_BUFFER_CAPACITY {
        place_insn(&mut mem, i * 4, stw_r3_off_r4((i * 4) as u16));
    }
    place_insn(&mut mem, BREAK_PC as usize, stw_r3_off_r4(BREAK_PC as u16));
    place_insn(&mut mem, BREAK_PC as usize + 4, 0x4400_0002); // sc
    place_insn(&mut mem, BREAK_PC as usize + 8, 0x4BFF_FFF8); // b BREAK_PC

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(4, 0x800);
    unit.set_break_pc(BREAK_PC, 1);

    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let first = unit.run_until_yield(Budget::new(200), &ctx, &mut effects);
    assert_eq!(first.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(
        unit.state().pc,
        BREAK_PC,
        "the full buffer leaves the store to retry"
    );
    assert_eq!(stores(&effects), STORE_BUFFER_CAPACITY);

    // The retry is the same hit, the one the skip covers: the store
    // retires and the block runs on to the syscall.
    let mut effects = Vec::new();
    let second = unit.run_until_yield(Budget::new(200), &ctx, &mut effects);
    assert_eq!(second.yield_reason, YieldReason::Syscall);
    assert_eq!(stores(&effects), 1);

    // The branch back is the second hit. The branch retires and its
    // block ends at the break; the next block fires it.
    let ctx = ExecutionContext::with_syscall_return(&mem, &[], 0);
    let third = unit.run_until_yield(Budget::new(200), &ctx, &mut Vec::new());
    assert_eq!(third.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, BREAK_PC);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let fourth = unit.run_until_yield(Budget::new(200), &ctx, &mut effects);
    assert_eq!(fourth.yield_reason, YieldReason::Fault);
    assert_eq!(fourth.fault, Some(FaultKind::Guest(FAULT_DEBUG_BREAK)));
    assert_eq!(fourth.local_diagnostics.pc, Some(BREAK_PC));
    assert!(effects.is_empty(), "the break's own block retired nothing");
    assert_eq!(unit.status(), UnitStatus::Faulted);
}

#[test]
fn a_break_inside_a_budget_window_commits_what_retired_and_then_fires() {
    const BREAK_PC: u64 = 8;
    let mut mem = GuestMemory::new(0x1000);
    place_insn(&mut mem, 0, stw_r3_off_r4(0));
    place_insn(&mut mem, 4, stw_r3_off_r4(4));
    place_insn(&mut mem, 8, stw_r3_off_r4(8));
    place_insn(&mut mem, 12, 0x4400_0002); // sc

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(4, 0x800);
    unit.set_break_pc(BREAK_PC, 0);

    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let first = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    assert_eq!(first.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(
        stores(&effects),
        2,
        "the two stores retired before the break are published"
    );
    assert_eq!(unit.state().pc, BREAK_PC);

    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(result.fault, Some(FaultKind::Guest(FAULT_DEBUG_BREAK)));
    assert_eq!(result.local_diagnostics.pc, Some(BREAK_PC));
    assert_eq!(result.consumed_cost, InstructionCost::ZERO);
    assert!(effects.is_empty());
    assert_eq!(unit.state().pc, BREAK_PC);
    assert_eq!(unit.status(), UnitStatus::Faulted);
}
