//! A fused compare and `bc` whose branch names its own slot costs what
//! the two unfused instructions cost.
//!
//! For such a pair, the next address is the address after its first
//! half. A fall-through of the first half reaches the same address. The
//! unit must retire the branch into the issue model as taken.

use crate::*;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::{Budget, InstructionCost};

use std::cell::RefCell;
use std::rc::Rc;

use crate::instruction::{FusedPair, PpuInstruction};
use crate::state::PpuState;

/// The pc of every fused pair the unit dispatched as one instruction.
#[derive(Default)]
struct PairDispatches(RefCell<Vec<u64>>);

impl PpuTap for PairDispatches {
    fn dispatch(&self, _unit: UnitId, insn: &PpuInstruction, state: &PpuState) {
        if insn.is_super_pair() {
            self.0.borrow_mut().push(state.pc);
        }
    }
}

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;

/// `cmpwi cr0, r3, 0`.
const CMPWI_R3: u32 = (11 << 26) | (3 << 16);
/// `cmpw cr0, r3, r4`.
const CMPW_R3_R4: u32 = (31 << 26) | (3 << 16) | (4 << 11);
/// `cmpdi cr0, r3, 0`: L (bit 10) is 1.
const CMPDI_R3: u32 = (11 << 26) | (1 << 21) | (3 << 16);
/// `cmpd cr0, r3, r4`: L (bit 10) is 1.
const CMPD_R3_R4: u32 = (31 << 26) | (1 << 21) | (3 << 16) | (4 << 11);
/// `cmplw cr0, r3, r4`: XO 32.
const CMPLW_R3_R4: u32 = (31 << 26) | (3 << 16) | (4 << 11) | (32 << 1);
/// Each compare form that fuses with a following `bc`.
const COMPARES: [u32; 5] = [CMPWI_R3, CMPW_R3_R4, CMPDI_R3, CMPD_R3_R4, CMPLW_R3_R4];
/// `beq .`: BO 12 (branch if the CR bit is set, no hint), BI 2 (CR0
/// EQ), displacement 0.
const BEQ_SELF: u32 = (16 << 26) | (12 << 21) | (2 << 16);
/// `addi r5, r5, 1`.
const ADDI_R5: u32 = (14 << 26) | (5 << 21) | (5 << 16) | 1;

/// What one batch left.
#[derive(Debug, PartialEq, Eq)]
struct Stop {
    reason: YieldReason,
    pc: Option<u64>,
    cost: InstructionCost,
    retired: u64,
    state: cellgov_exec::PpuFingerprint,
    state_pc: u64,
}

fn bytes(code: &[u32]) -> Vec<u8> {
    code.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// Runs `code` at 0 for four batches of `budget`, fused or by plain
/// decode, with r3 = `r3` and r4 = 0. Returns every batch's stop and
/// the pc of each fused pair the unit dispatched.
fn run(code: &[u32], fused: bool, budget: u64, r3: u64) -> (Vec<Stop>, Vec<u64>) {
    let bytes = bytes(code);
    let mut mem = GuestMemory::from_regions(vec![Region::new(
        0,
        FLAT_END as usize,
        "flat",
        PageSize::Page4K,
    )])
    .unwrap();
    mem.apply_commit(
        ByteRange::new(GuestAddr::new(0), bytes.len() as u64).unwrap(),
        &bytes,
    )
    .unwrap();
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    if fused {
        unit.set_instruction_shadow(shadow::PredecodedShadow::build(0, &bytes));
    }
    unit.state_mut().set_gpr(3, r3);
    let pairs = Rc::new(PairDispatches::default());
    unit.set_tap(pairs.clone());
    let mut stops = Vec::new();
    for _ in 0..4 {
        let r = unit.run_until_yield(
            Budget::new(budget),
            &ExecutionContext::new(&mem),
            &mut Vec::new(),
        );
        stops.push(Stop {
            reason: r.yield_reason,
            pc: r.local_diagnostics.pc,
            cost: r.consumed_cost,
            retired: unit.retired_instructions(),
            state: unit.state().fingerprint(),
            state_pc: unit.state().pc,
        });
    }
    let pairs = pairs.0.borrow().clone();
    (stops, pairs)
}

/// The pc of each fused pair a run of `budget` dispatches. The pair at 0
/// runs fused when the first batch can retire both of its slots. A plain
/// decode dispatches no pair.
fn expected_pairs(fused: bool, budget: u64) -> Vec<u64> {
    if fused && budget >= 2 {
        vec![0]
    } else {
        Vec::new()
    }
}

fn assert_fuses(code: &[u32]) {
    assert!(
        matches!(
            shadow::PredecodedShadow::build(0, &bytes(code)).get(0),
            Some(PpuInstruction::Fused(
                FusedPair::CmpwiBc { .. }
                    | FusedPair::CmpwBc { .. }
                    | FusedPair::CmpdiBc { .. }
                    | FusedPair::CmpdBc { .. }
                    | FusedPair::CmplwBc { .. }
            ))
        ),
        "precondition: the compare {:#010x} and the bc fuse at 0",
        code[0]
    );
}

/// The `beq .` spins at 4 while r3 is 0, so the budget bounds the loop.
/// [PPC-Book1 p:24 s:2.4] the target of a relative `bc` is its own
/// address plus the displacement.
#[test]
fn a_fused_compare_and_branch_to_its_own_slot_costs_what_it_costs_unfused() {
    for code in COMPARES.map(|cmp| [cmp, BEQ_SELF, ADDI_R5]) {
        assert_fuses(&code);
        for budget in [1, 2, 3, 8, 64] {
            let (fused, pairs) = run(&code, true, budget, 0);
            assert_eq!(pairs, expected_pairs(true, budget), "budget {budget}");
            let (plain, plain_pairs) = run(&code, false, budget, 0);
            assert_eq!(plain_pairs, expected_pairs(false, budget));
            assert_eq!(fused, plain, "budget {budget}");
            let last = fused.last().expect("four batches");
            assert_eq!(last.state_pc, 4, "the beq spins on itself");
            assert_eq!(
                last.retired,
                4 * budget,
                "every slot of every batch retired"
            );
        }
    }
}

#[test]
fn a_fused_compare_and_branch_to_its_own_slot_that_falls_through_costs_what_it_costs_unfused() {
    for code in COMPARES.map(|cmp| [cmp, BEQ_SELF, ADDI_R5]) {
        assert_fuses(&code);
        for budget in [1, 2, 3, 8] {
            let (fused, pairs) = run(&code, true, budget, 1);
            assert_eq!(pairs, expected_pairs(true, budget), "budget {budget}");
            assert_eq!(fused, run(&code, false, budget, 1).0, "budget {budget}");
        }
        let fused = run(&code, true, 3, 1).0;
        assert_eq!(fused[0].state_pc, 12, "the pair and the addi retired");
    }
}
