//! A batch that ends at a fused head runs the pair's second slot once,
//! even when a write between the batches drops the predecoded pair and
//! the next batch decodes the slot from memory.
//!
//! [PPC-Book1 p:17 s:2.2] each instruction runs once, in program order.

use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::*;
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodable")
}

/// A unit over `first; second; b .` with the first two words fused.
fn fused(first: I, second: I) -> (GuestMemory, PpuExecutionUnit) {
    let words = [
        word(first),
        word(second),
        word(I::B {
            offset: 0,
            aa: false,
            link: false,
        }),
    ];
    let mut code = Vec::new();
    for w in words {
        code.extend_from_slice(&w.to_be_bytes());
    }
    let mut mem = GuestMemory::new(0x100);
    let range = ByteRange::new(GuestAddr::new(0), code.len() as u64).expect("range");
    mem.apply_commit(range, &code).expect("mapped");
    let shadow = shadow::PredecodedShadow::build(0, &code);
    assert_eq!(
        shadow.get(4),
        Some(I::Consumed),
        "precondition: the pair fuses"
    );
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_instruction_shadow(shadow);
    (mem, unit)
}

/// `cmpwi cr0, r3, 0; bdnz +4` with CTR at 1: the branch decrements
/// CTR to zero and falls through to the word after the pair.
fn cmpwi_bdnz() -> (GuestMemory, PpuExecutionUnit) {
    let (mem, mut unit) = fused(
        I::Cmpwi {
            bf: 0,
            ra: 3,
            imm: 0,
        },
        I::Bc {
            bo: 16,
            bi: 0,
            offset: 4,
            aa: false,
            link: false,
        },
    );
    unit.state_mut().set_ctr(1);
    (mem, unit)
}

/// `std r5, 0x80(r4); std r6, 0x88(r4)`.
fn std_std() -> (GuestMemory, PpuExecutionUnit) {
    fused(
        I::Std {
            rs: 5,
            ra: 4,
            imm: 0x80,
        },
        I::Std {
            rs: 6,
            ra: 4,
            imm: 0x88,
        },
    )
}

/// The start address of every store in `effects`.
fn store_addrs(effects: &[Effect]) -> Vec<u64> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::SharedWriteIntent { range, .. } => Some(range.start().raw()),
            _ => None,
        })
        .collect()
}

/// Runs one batch of `budget` from the unit's pc, then drops the
/// predecoded pair, as a write inside it between batches does.
fn batch_then_write_inside_the_pair(
    unit: &mut PpuExecutionUnit,
    mem: &GuestMemory,
    budget: u64,
) -> Vec<Effect> {
    let ctx = ExecutionContext::new(mem);
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
    assert_eq!(r.yield_reason, YieldReason::BudgetExhausted);
    ExecutionUnit::invalidate_code(unit, 0, 8);
    effects
}

#[test]
fn a_fused_bdnz_decrements_ctr_once_when_the_pair_is_dropped_between_batches() {
    for first_budget in [1, 2] {
        let (mem, mut unit) = cmpwi_bdnz();
        batch_then_write_inside_the_pair(&mut unit, &mem, first_budget);
        batch_then_write_inside_the_pair(&mut unit, &mem, 1);
        assert_eq!(
            unit.state().ctr(),
            0,
            "first budget {first_budget}: one bdnz, one decrement"
        );
        assert_eq!(unit.state().pc, 8, "first budget {first_budget}");
    }
}

#[test]
fn a_fused_std_pair_stores_each_doubleword_once_when_dropped_between_batches() {
    for first_budget in [1, 2] {
        let (mem, mut unit) = std_std();
        let mut stores = store_addrs(&batch_then_write_inside_the_pair(
            &mut unit,
            &mem,
            first_budget,
        ));
        stores.extend(store_addrs(&batch_then_write_inside_the_pair(
            &mut unit, &mem, 1,
        )));
        assert_eq!(stores, vec![0x80, 0x88], "first budget {first_budget}");
        assert_eq!(unit.state().pc, 8, "first budget {first_budget}");
    }
}
