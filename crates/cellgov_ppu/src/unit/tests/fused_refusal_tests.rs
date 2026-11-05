//! A refusal inside a fused pair's second half stops the unit where the
//! unfused instructions stop it: the first half retires, and the second
//! refuses at its own address with the first half's result in place.
//!
//! [PPC-Book3 p:61 s:5.2] when an interrupt occurs, every preceding
//! instruction has completed and no subsequent instruction has begun.
//! [PPC-Book1 p:17 s:2.2] the processor appears to execute one
//! instruction at a time, so a pair's first half is a preceding
//! instruction of its second.

use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

/// The end of the one region, at 0.
const FLAT_END: u64 = 0x1000;
/// An address no region holds.
const UNMAPPED: u64 = 0x8000;
/// The base of the stores that fill the store buffer.
const FILL_BASE: u16 = 0x800;
/// The entries the store buffer holds.
const STORE_BUFFER_ENTRIES: usize = 64;

/// `addi r3, r3, 1`.
const ADDI_R3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `li r6, 7`.
const LI_R6_7: u32 = (14 << 26) | (6 << 21) | 7;
/// `stw r6, 0(r5)`.
const STW_R6_R5: u32 = (36 << 26) | (6 << 21) | (5 << 16);
/// `mflr r0`.
const MFLR_R0: u32 = (31 << 26) | (8 << 16) | (339 << 1);
/// `stw r0, 0(r5)`.
const STW_R0_R5: u32 = (36 << 26) | (5 << 16);
/// `std r0, 0(r5)`.
const STD_R0_R5: u32 = (62 << 26) | (5 << 16);
/// `std r6, 0(r5)`.
const STD_R6_R5: u32 = (62 << 26) | (6 << 21) | (5 << 16);
/// `std r7, 8(r5)`.
const STD_R7_R5_8: u32 = (62 << 26) | (7 << 21) | (5 << 16) | 8;

/// `stw r3, d(0)`.
fn stw_r3_abs(d: u16) -> u32 {
    (36 << 26) | (3 << 21) | u32::from(d)
}

/// `n` stores to distinct words from [`FILL_BASE`] on.
fn fill(n: usize) -> Vec<u32> {
    (0..n)
        .map(|i| stw_r3_abs(FILL_BASE + 4 * i as u16))
        .collect()
}

/// One batch, as the unit leaves it.
#[derive(Debug, PartialEq, Eq)]
struct Batch {
    reason: YieldReason,
    pc: Option<u64>,
    faulting_ea: Option<u64>,
    cost: u64,
    retired: u64,
    state: cellgov_exec::PpuFingerprint,
    state_pc: u64,
    memory: Vec<u8>,
}

/// A program at 0 and the registers it starts with.
struct Program {
    code: Vec<u32>,
    r5: u64,
    lr: u64,
    /// The break pc and the hits it skips.
    break_at: Option<(u64, u32)>,
}

impl Program {
    fn new(code: Vec<u32>, r5: u64) -> Self {
        Self {
            code,
            r5,
            lr: 0x1234_5678_9ABC_DEF0,
            break_at: None,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        self.code.iter().flat_map(|w| w.to_be_bytes()).collect()
    }

    /// The shadow slot of the word at `pc`.
    fn fused_at(&self, pc: u64) -> Option<instruction::PpuInstruction> {
        shadow::PredecodedShadow::build(0, &self.bytes()).get(pc)
    }

    /// Runs the program, through the fused shadow or by plain decode,
    /// in batches of `budget` until a batch faults, for at most eight
    /// batches. Each batch that ends otherwise commits its stores.
    fn run(&self, fused: bool, budget: u64) -> Vec<Batch> {
        let mut mem = GuestMemory::from_regions(vec![Region::new(
            0,
            FLAT_END as usize,
            "flat",
            PageSize::Page4K,
        )])
        .unwrap();
        for (i, word) in self.code.iter().enumerate() {
            let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).unwrap();
            mem.apply_commit(range, &word.to_be_bytes()).unwrap();
        }
        let mut unit = PpuExecutionUnit::new(UnitId::new(0));
        if fused {
            unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &self.bytes()));
        }
        unit.state_mut().set_gpr(3, 1);
        unit.state_mut().set_gpr(5, self.r5);
        unit.state_mut().set_gpr(6, 0x66);
        unit.state_mut().set_gpr(7, 0x77);
        unit.state_mut().set_lr(self.lr);
        if let Some((pc, skip)) = self.break_at {
            unit.set_break_pc(pc, skip);
        }
        let mut batches = Vec::new();
        for _ in 0..8 {
            let ctx = ExecutionContext::new(&mem);
            let mut effects = Vec::new();
            let r = unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
            if r.yield_reason != YieldReason::Fault {
                for effect in effects {
                    if let Effect::SharedWriteIntent { range, bytes, .. } = effect {
                        mem.apply_commit(range, bytes.bytes()).unwrap();
                    }
                }
            }
            batches.push(Batch {
                reason: r.yield_reason,
                pc: r.local_diagnostics.pc,
                faulting_ea: r.local_diagnostics.faulting_ea,
                cost: r.consumed_cost.raw(),
                retired: unit.retired_instructions(),
                state: unit.state().fingerprint(),
                state_pc: unit.state().pc,
                memory: mem.as_bytes().to_vec(),
            });
            if r.yield_reason == YieldReason::Fault {
                break;
            }
        }
        batches
    }
}

/// Asserts the pair at `head` fuses as `kind`, then that the fused run
/// and the plain run stop the same way at every budget.
fn assert_stops_as_unfused(program: &Program, head: u64, kind: &str) -> Vec<Batch> {
    let fused = program.fused_at(head);
    assert!(
        fused.is_some_and(|insn| insn.is_super_pair()),
        "precondition: {kind} fuses at {head:#x}, got {fused:?}"
    );
    let plain = program.run(false, 256);
    for budget in [1, 2, 3, 256] {
        assert_eq!(
            program.run(true, budget),
            program.run(false, budget),
            "{kind} at budget {budget}"
        );
    }
    plain
}

#[test]
fn each_store_pair_whose_second_store_faults_retires_its_first_half_first() {
    for (kind, first, second) in [
        ("li; stw", LI_R6_7, STW_R6_R5),
        ("mflr; stw", MFLR_R0, STW_R0_R5),
        ("mflr; std", MFLR_R0, STD_R0_R5),
    ] {
        let program = Program::new(vec![ADDI_R3, first, second, ADDI_R3], UNMAPPED);

        let plain = assert_stops_as_unfused(&program, 4, kind);

        let last = plain.last().expect("a batch");
        assert_eq!(last.reason, YieldReason::Fault, "{kind}");
        assert_eq!(
            last.pc,
            Some(8),
            "{kind}: the store faults at its own address"
        );
        assert_eq!(last.faulting_ea, Some(UNMAPPED), "{kind}");
        assert_eq!(last.retired, 2, "{kind}: addi and the first half retired");
    }
}

#[test]
fn a_std_pair_whose_second_doubleword_runs_off_the_region_commits_the_first() {
    let program = Program::new(vec![ADDI_R3, STD_R6_R5, STD_R7_R5_8, ADDI_R3], FLAT_END - 8);

    let plain = assert_stops_as_unfused(&program, 4, "std; std");

    let last = plain.last().expect("a batch");
    assert_eq!(last.reason, YieldReason::Fault);
    assert_eq!(last.pc, Some(8));
    assert_eq!(last.faulting_ea, Some(FLAT_END));
    let first = &last.memory[(FLAT_END - 8) as usize..];
    assert_eq!(
        first,
        0x66u64.to_be_bytes(),
        "the first doubleword committed"
    );
}

#[test]
fn a_full_store_buffer_stops_a_store_pair_after_its_first_half() {
    let mut code = fill(STORE_BUFFER_ENTRIES);
    let head = 4 * code.len() as u64;
    code.extend([LI_R6_7, STW_R6_R5, ADDI_R3]);
    let program = Program::new(code, 0x900);

    let plain = assert_stops_as_unfused(&program, head, "li; stw");

    let first = &plain[0];
    assert_eq!(first.reason, YieldReason::BudgetExhausted);
    assert_eq!(first.state_pc, head + 4, "the store waits for the buffer");
    assert_eq!(first.retired, STORE_BUFFER_ENTRIES as u64 + 1, "li retired");
}

#[test]
fn a_std_pair_one_entry_short_of_a_full_buffer_stages_one_store_per_doubleword() {
    let mut code = fill(STORE_BUFFER_ENTRIES - 1);
    let head = 4 * code.len() as u64;
    code.extend([STD_R6_R5, STD_R7_R5_8, ADDI_R3]);
    let program = Program::new(code, 0x900);

    let plain = assert_stops_as_unfused(&program, head, "std; std");

    let first = &plain[0];
    assert_eq!(first.reason, YieldReason::BudgetExhausted);
    assert_eq!(
        first.state_pc,
        head + 4,
        "the second std waits for the buffer"
    );
    assert_eq!(
        first.memory[0x900..0x908],
        0x66u64.to_be_bytes(),
        "the first std committed in the first batch"
    );
    assert_eq!(first.memory[0x908..0x910], [0; 8]);
    let next = &plain[1];
    assert_eq!(next.memory[0x908..0x910], 0x77u64.to_be_bytes());
}

#[test]
fn a_break_on_a_fused_second_slot_stops_with_only_the_first_half_applied() {
    let mut program = Program::new(vec![ADDI_R3, MFLR_R0, STW_R0_R5, ADDI_R3], 0x900);
    program.break_at = Some((8, 0));

    let plain = assert_stops_as_unfused(&program, 4, "mflr; stw");

    let last = plain.last().expect("a batch");
    assert_eq!(last.reason, YieldReason::Fault);
    assert_eq!(last.pc, Some(8));
    assert_eq!(last.memory[0x900..0x904], [0; 4], "the store never ran");
}

#[test]
fn a_skipped_break_on_a_refused_pair_head_is_spent_once() {
    let mut code = fill(STORE_BUFFER_ENTRIES);
    let full_head = 4 * code.len() as u64;
    code.extend([LI_R6_7, STW_R6_R5, ADDI_R3]);
    let full = Program::new(code, 0x900);
    let faulting = Program::new(vec![ADDI_R3, MFLR_R0, STW_R0_R5, ADDI_R3], UNMAPPED);
    // The full-buffer program stops at the all-zero word past its end,
    // which does not decode.
    for (kind, mut program, head, stop) in [
        (
            "li; stw into a full buffer",
            full,
            full_head,
            full_head + 12,
        ),
        ("mflr; stw to an unmapped word", faulting, 4, 8),
    ] {
        program.break_at = Some((head, 1));

        let plain = assert_stops_as_unfused(&program, head, kind);

        let last = plain.last().expect("a batch");
        assert_eq!(last.reason, YieldReason::Fault, "{kind}");
        assert_eq!(
            last.pc,
            Some(stop),
            "{kind}: the one skip lets the head pass, so no break fires"
        );
    }
}
