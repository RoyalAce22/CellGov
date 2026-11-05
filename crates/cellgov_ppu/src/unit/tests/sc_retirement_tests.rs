//! `sc` retires at its yield: the retirement count, the per-step hash
//! stream, the full-state window and break skips all include it.

use crate::*;
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

const LI_R3_5: u32 = 0x3860_0005;
const SC: u32 = 0x4400_0002;
const LI_R4_1: u32 = 0x3880_0001;
const B_BACK_8: u32 = 0x4BFF_FFF8;

/// The return value the runtime hands back for the syscall.
const RETURN: u64 = 7;

fn program(words: &[u32]) -> GuestMemory {
    let mut mem = GuestMemory::new(0x100);
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &w.to_be_bytes()).unwrap();
    }
    mem
}

#[test]
fn a_batch_that_ends_in_sc_counts_and_hashes_it() {
    let mem = program(&[LI_R3_5, SC, LI_R4_1]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));

    let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
    let first = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(first.yield_reason, YieldReason::Syscall);
    assert_eq!(unit.retired_instructions(), 2);
    let at_yield = unit.state().state_hash();
    let hashes = unit.drain_retired_state_hashes();
    assert_eq!(hashes, [(0, hashes[0].1), (4, at_yield)]);

    // The return value and the step past `sc` land in the next batch's
    // first record.
    let ctx = ExecutionContext::with_syscall_return(&mem, &[], RETURN).with_trace_per_step(true);
    unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    assert_eq!(unit.retired_instructions(), 3);
    assert_eq!(unit.state().gpr[3], RETURN);
    let after = unit.drain_retired_state_hashes();
    assert_eq!(after, [(8, unit.state().state_hash())]);
    assert_ne!(after[0].1, at_yield);
}

#[test]
fn a_full_state_window_records_sc_at_its_retirement_index() {
    let mem = program(&[LI_R3_5, SC, LI_R4_1]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_full_state_window(Some((1, 2)));

    let ctx = ExecutionContext::new(&mem);
    unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    let at_sc = unit.drain_retired_state_full();
    assert_eq!(at_sc.len(), 1);
    assert_eq!((at_sc[0].0, at_sc[0].1), (1, 4));
    assert_eq!(at_sc[0].2.gpr[3], 5, "the state at the yield");

    let ctx = ExecutionContext::with_syscall_return(&mem, &[], RETURN);
    unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());
    let next = unit.drain_retired_state_full();
    assert_eq!(next.len(), 1);
    assert_eq!((next[0].0, next[0].1), (2, 8));
    assert_eq!(next[0].2.gpr[3], RETURN);
}

#[test]
fn a_break_on_sc_fires_after_the_retirements_its_skip_passed() {
    // `li r3, 5; sc; b -8`: the loop visits `sc` once per pass.
    let mem = program(&[LI_R3_5, SC, B_BACK_8]);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_break_pc(4, 1);

    let ctx = ExecutionContext::new(&mem);
    let first = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(first.yield_reason, YieldReason::Syscall);

    // The second visit ends the batch at the break; the next one fires.
    let ctx = ExecutionContext::with_syscall_return(&mem, &[], RETURN);
    let second = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(second.yield_reason, YieldReason::BudgetExhausted);
    let ctx = ExecutionContext::new(&mem);
    let third = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(third.fault, Some(FaultKind::Guest(FAULT_DEBUG_BREAK)));
    assert_eq!(third.local_diagnostics.pc, Some(4));
    // li, sc, b, li.
    assert_eq!(unit.retired_instructions(), 4);
}
