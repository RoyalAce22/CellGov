//! A fault never undoes an instruction that retired before it. The
//! batch commits its retired instructions, and the next batch runs the
//! instruction that faults with nothing retired ahead of it.

use crate::*;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, UnitStatus, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

const UNMAPPED_EA: u64 = 0xFFFF_0000;

fn place(mem: &mut GuestMemory, offset: usize, raw: u32) {
    let range = ByteRange::new(GuestAddr::new(offset as u64), 4).unwrap();
    mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
}

fn read_u32(mem: &GuestMemory, addr: u64) -> u32 {
    let bytes = &mem.as_bytes()[addr as usize..addr as usize + 4];
    u32::from_be_bytes(bytes.try_into().unwrap())
}

/// `addi rT, rT, 1`.
fn addi_inc(rt: u32) -> u32 {
    (14 << 26) | (rt << 21) | (rt << 16) | 1
}

/// `stw rS, d(0)`.
fn stw_abs(rs: u32, d: u16) -> u32 {
    (36 << 26) | (rs << 21) | u32::from(d)
}

/// `mtlr rS`.
fn mtlr(rs: u32) -> u32 {
    (31 << 26) | (rs << 21) | (8 << 16) | (467 << 1)
}

/// `mtctr rS`.
fn mtctr(rs: u32) -> u32 {
    (31 << 26) | (rs << 21) | (9 << 16) | (467 << 1)
}

/// `lwz r4, 0(r5)`; r5 holds `UNMAPPED_EA`.
const LWZ_UNMAPPED: u32 = (32 << 26) | (4 << 21) | (5 << 16);

/// What a unit run to its fault left behind.
struct FaultRun {
    fault: ExecutionStepResult,
    unit: PpuExecutionUnit,
    mem: GuestMemory,
    cost: u64,
    batches: usize,
    hashes: Vec<(u64, u64)>,
}

/// Runs `unit` over `mem` in batches of `budget` until a batch yields
/// anything but an exhausted budget. Each exhausted batch commits its
/// stores. Panics if that is not a fault within 64 batches.
fn run_to_fault(mut unit: PpuExecutionUnit, mut mem: GuestMemory, budget: u64) -> FaultRun {
    let mut cost = 0;
    let mut hashes = Vec::new();
    for batch in 1..=64 {
        let ctx = ExecutionContext::new(&mem).with_trace_per_step(true);
        let mut effects = Vec::new();
        let result = unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
        cost += result.consumed_cost.raw();
        hashes.extend(unit.drain_retired_state_hashes());
        match result.yield_reason {
            YieldReason::Fault => {
                return FaultRun {
                    fault: result,
                    unit,
                    mem,
                    cost,
                    batches: batch,
                    hashes,
                };
            }
            YieldReason::BudgetExhausted => {
                for effect in effects {
                    if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
                        mem.apply_commit(range, bytes.bytes()).unwrap();
                    }
                }
            }
            other => panic!("batch {batch} yielded {other:?}"),
        }
    }
    panic!("no fault within 64 batches");
}

/// `addi r3; stw r3,0x80; mtlr r3; mtctr r3; stw r3,0x84; lwz` faults
/// at 20 after five retire.
fn prefix_then_fault() -> (PpuExecutionUnit, GuestMemory) {
    let mut mem = GuestMemory::new(256);
    for (i, raw) in [
        addi_inc(3),
        stw_abs(3, 0x80),
        mtlr(3),
        mtctr(3),
        stw_abs(3, 0x84),
        LWZ_UNMAPPED,
    ]
    .into_iter()
    .enumerate()
    {
        place(&mut mem, i * 4, raw);
    }
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(3, 10);
    unit.state_mut().set_gpr(5, UNMAPPED_EA);
    (unit, mem)
}

#[test]
fn the_batch_commits_what_retired_before_the_fault() {
    let (mut unit, mem) = prefix_then_fault();
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();

    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);

    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert!(r.consumed_cost.raw() > 0, "the retired prefix costs time");
    assert_eq!(r.local_diagnostics.pc, Some(16), "the last retired pc");
    assert_eq!(unit.state().pc, 20, "PC stays on the faulting load");
    assert_eq!(unit.state().gpr[3], 11);
    assert_eq!(unit.state().lr(), 11);
    assert_eq!(unit.state().ctr(), 11);
    assert_eq!(unit.status(), UnitStatus::Runnable);
    assert_eq!(unit.retired_instructions(), 5);
    let stores = effects
        .iter()
        .filter(|e| matches!(e, Effect::SharedWriteIntent { .. }))
        .count();
    assert_eq!(stores, 2, "both stores before the fault are published");
}

#[test]
fn the_next_batch_faults_with_nothing_retired_ahead_of_it() {
    let (unit, mem) = prefix_then_fault();

    let run = run_to_fault(unit, mem, 64);

    assert_eq!(run.batches, 2);
    assert_eq!(
        run.fault.fault,
        Some(FaultKind::Guest(FAULT_DATA_STORAGE | 0x4000))
    );
    assert_eq!(run.fault.local_diagnostics.pc, Some(20));
    assert_eq!(run.fault.local_diagnostics.faulting_ea, Some(UNMAPPED_EA));
    assert_eq!(run.fault.consumed_cost.raw(), 0);
    assert_eq!(run.unit.status(), UnitStatus::Faulted);
    assert_eq!(run.unit.state().pc, 20);
    assert_eq!(run.unit.state().gpr[3], 11);
    assert_eq!(run.unit.state().mem_fault_arm_entries, 1);
    assert_eq!(read_u32(&run.mem, 0x80), 11);
    assert_eq!(read_u32(&run.mem, 0x84), 11);
    assert_eq!(run.hashes.len(), 5, "the retired prefix stays in the trace");
}

#[test]
fn the_state_at_a_fault_does_not_depend_on_the_budget() {
    let runs: Vec<FaultRun> = [1, 2, 3, 256]
        .into_iter()
        .map(|budget| {
            let (unit, mem) = prefix_then_fault();
            run_to_fault(unit, mem, budget)
        })
        .collect();

    let first = &runs[0];
    for (run, budget) in runs.iter().zip([1, 2, 3, 256]) {
        assert_eq!(
            run.unit.state().gpr.as_array(),
            first.unit.state().gpr.as_array(),
            "budget {budget}"
        );
        assert_eq!(
            run.unit.state().lr(),
            first.unit.state().lr(),
            "budget {budget}"
        );
        assert_eq!(
            run.unit.state().ctr(),
            first.unit.state().ctr(),
            "budget {budget}"
        );
        assert_eq!(run.unit.state().pc, 20, "budget {budget}");
        assert_eq!(run.mem.as_bytes(), first.mem.as_bytes(), "budget {budget}");
        assert_eq!(run.cost, first.cost, "budget {budget}");
        assert_eq!(run.hashes, first.hashes, "budget {budget}");
        assert_eq!(run.fault.local_diagnostics.pc, Some(20), "budget {budget}");
    }
}

#[test]
fn a_decode_fault_after_retired_instructions_commits_them_first() {
    let mut mem = GuestMemory::new(256);
    place(&mut mem, 0, addi_inc(3));
    place(&mut mem, 4, stw_abs(3, 0x80));
    // The all-zero word at 8 does not decode.
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(3, 10);

    let run = run_to_fault(unit, mem, 64);

    assert_eq!(run.batches, 2);
    assert_eq!(run.fault.local_diagnostics.pc, Some(8));
    assert_eq!(run.unit.state().gpr[3], 11);
    assert_eq!(run.unit.state().pc, 8);
    assert_eq!(read_u32(&run.mem, 0x80), 11);
}

#[test]
fn a_fetch_past_the_text_after_retired_instructions_commits_them_first() {
    let mut mem = GuestMemory::new(8);
    place(&mut mem, 0, addi_inc(3));
    place(&mut mem, 4, addi_inc(4));
    let unit = PpuExecutionUnit::new(UnitId::new(0));

    let run = run_to_fault(unit, mem, 64);

    assert_eq!(
        run.fault.fault,
        Some(FaultKind::Guest(FAULT_INSTRUCTION_STORAGE | 0x4000))
    );
    assert_eq!(run.fault.local_diagnostics.pc, Some(8));
    assert_eq!(run.unit.state().gpr[3], 1);
    assert_eq!(run.unit.state().gpr[4], 1);
}

#[test]
fn a_break_after_retired_instructions_commits_them_and_then_stops() {
    let (mut unit, mem) = prefix_then_fault();
    unit.set_break_pc(12, 0);

    let run = run_to_fault(unit, mem, 64);

    assert_eq!(run.batches, 2);
    assert_eq!(run.fault.fault, Some(FaultKind::Guest(FAULT_DEBUG_BREAK)));
    assert_eq!(run.fault.local_diagnostics.pc, Some(12));
    assert_eq!(run.unit.state().lr(), 11, "mtlr retired before the break");
    assert_eq!(read_u32(&run.mem, 0x80), 11);
}

#[test]
fn a_skipped_break_on_a_faulting_instruction_is_spent_once() {
    let (mut unit, mem) = prefix_then_fault();
    // The load at 20 is visited twice: once behind the prefix, once
    // alone. Only the second visit counts.
    unit.set_break_pc(20, 1);

    let run = run_to_fault(unit, mem, 64);

    assert_eq!(run.batches, 2);
    assert_eq!(
        run.fault.fault,
        Some(FaultKind::Guest(FAULT_DATA_STORAGE | 0x4000))
    );
    assert_eq!(run.unit.state().pc, 20);
}

#[test]
fn a_fused_pair_whose_store_faults_retires_its_first_instruction() {
    // li r6,7; stw r6,-2(r8) with r8 = 0: the store's last byte lies
    // past the end of the address space.
    let li_r6_7: u32 = (14 << 26) | (6 << 21) | 7;
    let stw_r6_wrap: u32 = (36 << 26) | (6 << 21) | (8 << 16) | 0xFFFE;
    let mut mem = GuestMemory::new(256);
    place(&mut mem, 0, addi_inc(3));
    place(&mut mem, 4, li_r6_7);
    place(&mut mem, 8, stw_r6_wrap);
    let mut code = [0u8; 12];
    code[..4].copy_from_slice(&addi_inc(3).to_be_bytes());
    code[4..8].copy_from_slice(&li_r6_7.to_be_bytes());
    code[8..].copy_from_slice(&stw_r6_wrap.to_be_bytes());
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert!(
        matches!(
            shadow.get(4),
            Some(instruction::PpuInstruction::LiStw { .. })
        ),
        "precondition: li and stw fuse"
    );
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);

    let run = run_to_fault(unit, mem, 64);

    assert_eq!(run.fault.local_diagnostics.pc, Some(8), "the store faults");
    assert_eq!(run.unit.state().pc, 8);
    assert_eq!(run.unit.state().gpr[3], 1);
    assert_eq!(run.unit.state().gpr[6], 7, "li retired before the store");
    assert_eq!(run.unit.retired_instructions(), 2);
}

#[test]
fn a_fused_pair_whose_load_faults_faults_at_its_first_slot() {
    // lwz r0,0(r5); mtlr r0 with r5 = UNMAPPED_EA: the first half faults.
    let lwz_r0_r5: u32 = (32 << 26) | (5 << 16);
    let mut mem = GuestMemory::new(256);
    place(&mut mem, 0, addi_inc(3));
    place(&mut mem, 4, lwz_r0_r5);
    place(&mut mem, 8, mtlr(0));
    let mut code = [0u8; 12];
    code[..4].copy_from_slice(&addi_inc(3).to_be_bytes());
    code[4..8].copy_from_slice(&lwz_r0_r5.to_be_bytes());
    code[8..].copy_from_slice(&mtlr(0).to_be_bytes());
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert!(
        matches!(
            shadow.get(4),
            Some(instruction::PpuInstruction::LwzMtlr { .. })
        ),
        "precondition: lwz and mtlr fuse"
    );
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    unit.state_mut().set_gpr(5, UNMAPPED_EA);
    unit.state_mut().set_lr(0x1234);

    let run = run_to_fault(unit, mem, 64);

    assert_eq!(run.batches, 2);
    assert_eq!(
        run.fault.fault,
        Some(FaultKind::Guest(FAULT_DATA_STORAGE | 0x4000))
    );
    assert_eq!(run.fault.local_diagnostics.pc, Some(4), "the load faults");
    assert_eq!(run.unit.state().pc, 4);
    assert_eq!(run.unit.state().lr(), 0x1234, "mtlr never ran");
    assert_eq!(run.unit.retired_instructions(), 1);
}

#[test]
fn a_load_multiple_that_faults_part_way_publishes_none_of_its_reads() {
    // lmw r30,0(r5) with r5 = 252: the word at 252 loads and records a
    // read, then the word at 256 lies past the one region.
    let lmw_r30_r5: u32 = (46 << 26) | (30 << 21) | (5 << 16);
    let mut mem = GuestMemory::new(256);
    place(&mut mem, 0, addi_inc(3));
    place(&mut mem, 4, lmw_r30_r5);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_gpr(5, 252);
    let ctx = ExecutionContext::new(&mem);
    let reads_252 = |effects: &[Effect]| {
        effects
            .iter()
            .filter(|e| match e {
                Effect::SharedReadIntent { range, .. } => {
                    let start = range.start().raw();
                    start <= 252 && 252 < start + range.length()
                }
                _ => false,
            })
            .count()
    };

    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, 4);
    assert_eq!(unit.state().gpr[3], 1);
    assert_eq!(reads_252(&effects), 0, "{effects:?}");

    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(r.fault, Some(FaultKind::Guest(FAULT_DATA_STORAGE | 0x4000)));
    assert_eq!(r.local_diagnostics.pc, Some(4));
    assert_eq!(unit.state().gpr[30], 0, "lmw wrote no register");
    assert!(effects.is_empty(), "{effects:?}");
}

#[test]
fn a_block_that_ends_at_a_fault_publishes_only_the_text_it_retired() {
    let (mut unit, mem) = prefix_then_fault();
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();

    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);

    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    let fetched: Vec<(u64, u64)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } => Some((range.start().raw(), range.length())),
            _ => None,
        })
        .collect();
    assert_eq!(
        fetched,
        [(0, 20)],
        "the five retired words, not the load at 20"
    );
}

#[test]
fn a_fetch_past_the_end_of_the_address_space_behind_a_prefix_publishes_nothing_for_it() {
    // `bclr` to the last word of the address space, which no region maps.
    let mut mem = GuestMemory::new(256);
    place(&mut mem, 0, addi_inc(3));
    place(&mut mem, 4, 0x4E80_0020); // blr
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().set_lr(u64::MAX - 3);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();

    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);

    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(unit.state().pc, u64::MAX - 3);
    let fetched: Vec<u64> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } => Some(range.start().raw()),
            _ => None,
        })
        .collect();
    assert_eq!(fetched, [0]);
}
