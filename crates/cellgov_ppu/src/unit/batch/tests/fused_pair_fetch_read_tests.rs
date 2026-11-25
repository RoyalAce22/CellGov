//! A block that runs a fused pair publishes the fetch reads and the
//! effects of the same block run unfused.

use crate::instruction::{FusedPair, PpuInstruction};
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, PageSize, Region};
use cellgov_time::Budget;

/// The end of the one region, which starts at 0.
const FLAT_END: u64 = 0x1000;
/// The stack pointer the block starts with.
const STACK: u64 = 0x800;

/// `std rS, ds(rA)`.
fn std(rs: u32, ra: u32, ds: i16) -> u32 {
    (62 << 26) | (rs << 21) | (ra << 16) | u32::from(ds as u16)
}

/// `lwz rT, d(rA)`.
fn lwz(rt: u32, ra: u32, d: i16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d as u16)
}

/// `li rT, 0`.
fn li_zero(rt: u32) -> u32 {
    (14 << 26) | (rt << 21)
}

/// `std r5,-16(r1) ; lwz r6,8(r1) ; li r3,0 ; li r4,0`. The first two
/// words fuse.
fn code() -> [u32; 4] {
    [std(5, 1, -16), lwz(6, 1, 8), li_zero(3), li_zero(4)]
}

fn code_bytes() -> Vec<u8> {
    code().iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// Runs [`code`] at 0 in one batch of budget 4, with a predecoded shadow
/// when `fused`, and returns the effects of the batch.
fn run_block(fused: bool) -> Vec<Effect> {
    let bytes = code_bytes();
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
    unit.state_mut().set_gpr(1, STACK);
    unit.state_mut().set_gpr(5, 0x1122_3344_5566_7788);
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(4), &ExecutionContext::new(&mem), &mut effects);
    assert_eq!(
        r.yield_reason,
        YieldReason::BudgetExhausted,
        "fused {fused}"
    );
    assert_eq!(unit.retired_instructions(), 4, "fused {fused}");
    assert_eq!(unit.state().pc, 16, "fused {fused}");
    effects
}

/// The `(start, length)` of each read intent in `effects` that starts
/// inside [`code`].
fn text_reads(effects: &[Effect]) -> Vec<(u64, u64)> {
    let text_end = 4 * code().len() as u64;
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedReadIntent { range, .. } => Some((range.start().raw(), range.length())),
            _ => None,
        })
        .filter(|&(start, _)| start < text_end)
        .collect()
}

fn assert_fuses() {
    assert!(
        matches!(
            shadow::PredecodedShadow::build(0, &code_bytes()).get(0),
            Some(PpuInstruction::Fused(FusedPair::StdLwz { .. }))
        ),
        "precondition: the std and the lwz fuse at 0"
    );
}

#[test]
fn a_fused_pair_publishes_a_fetch_read_of_both_its_words() {
    assert_fuses();
    let unfused = text_reads(&run_block(false));
    assert_eq!(
        unfused,
        [(0, 16)],
        "the unfused block fetches all four words"
    );
    assert_eq!(text_reads(&run_block(true)), unfused);
}

#[test]
fn a_fused_pair_block_publishes_the_effects_of_the_unfused_block() {
    assert_fuses();
    assert_eq!(run_block(true), run_block(false));
}
