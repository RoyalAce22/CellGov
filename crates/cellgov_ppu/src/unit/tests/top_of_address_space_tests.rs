//! A branch to the last word of the 64-bit address space faults the
//! thread with the instruction storage interrupt. No fetch run that the unit
//! publishes wraps past the top of the address space.
//!
//! [PPC-Book1 p:15 s:1.12.2] instructions are four bytes and
//! word-aligned, and each sequential fetch computes an effective address.

use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::*;
use cellgov_effects::{Effect, FaultKind};
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionStepResult, ExecutionUnit, YieldReason};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::Budget;

/// The last word-aligned address of the 64-bit address space.
const LAST_WORD: u64 = 0xFFFF_FFFF_FFFF_FFFC;

/// A unit at pc 0 over memory holding `branch`.
fn unit_over(branch: I) -> (PpuExecutionUnit, GuestMemory) {
    let mut mem = GuestMemory::new(0x100);
    let word = encode(&branch).expect("encodable");
    let range = ByteRange::new(GuestAddr::new(0), 4).expect("range");
    mem.apply_commit(range, &word.to_be_bytes())
        .expect("mapped");
    (PpuExecutionUnit::new(UnitId::new(0)), mem)
}

/// Runs `unit` in batches until one yields anything but an exhausted
/// budget, and returns that batch's result with every effect the
/// batches published.
fn run_to_yield(
    unit: &mut PpuExecutionUnit,
    mem: &GuestMemory,
    budget: u64,
) -> (ExecutionStepResult, Vec<Effect>) {
    let mut all = Vec::new();
    for _ in 0..8 {
        let ctx = ExecutionContext::new(mem);
        let mut effects = Vec::new();
        let result = unit.run_until_yield(Budget::new(budget), &ctx, &mut effects);
        all.extend(effects);
        if result.yield_reason != YieldReason::BudgetExhausted {
            return (result, all);
        }
    }
    panic!("no yield other than an exhausted budget within 8 batches");
}

/// Asserts that the unit `setup` builds faults with the PC-out-of-range
/// fault at [`LAST_WORD`] and publishes only the fetch of the branch at 0.
///
/// Each budget runs with no shadow and with the predecoded shadow that a
/// title boot installs over the text.
fn assert_faults_at_the_last_word(setup: impl Fn() -> (PpuExecutionUnit, GuestMemory), what: &str) {
    for (budget, shadowed) in [1, 2, 64].into_iter().flat_map(|b| [(b, false), (b, true)]) {
        let what = format!("{what}, shadowed {shadowed}");
        let (mut unit, mem) = setup();
        if shadowed {
            unit.set_instruction_shadow(crate::shadow::PredecodedShadow::build(0, mem.as_bytes()));
        }
        let (result, effects) = run_to_yield(&mut unit, &mem, budget);
        assert_eq!(
            result.yield_reason,
            YieldReason::Fault,
            "{what}, budget {budget}"
        );
        assert_eq!(
            result.fault,
            Some(FaultKind::Guest(FAULT_INSTRUCTION_STORAGE | 0x4000)),
            "{what}, budget {budget}"
        );
        assert_eq!(unit.state().pc, LAST_WORD, "{what}, budget {budget}");
        let fetches: Vec<(u64, u64)> = effects
            .iter()
            .filter_map(|e| match e {
                Effect::SharedReadIntent { range, .. } => {
                    Some((range.start().raw(), range.length()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(fetches, vec![(0, 4)], "{what}, budget {budget}");
    }
}

#[test]
fn bctr_to_the_last_word_faults_the_thread() {
    let setup = || {
        let (mut unit, mem) = unit_over(I::Bcctr {
            bo: 20,
            bi: 0,
            link: false,
        });
        unit.state_mut().set_ctr((-4i64) as u64);
        (unit, mem)
    };
    assert_faults_at_the_last_word(setup, "bctr, CTR = -4");
}

#[test]
fn blr_to_the_last_word_faults_the_thread() {
    let setup = || {
        let (mut unit, mem) = unit_over(I::Bclr {
            bo: 20,
            bi: 0,
            link: false,
        });
        unit.state_mut().set_lr(u64::MAX);
        (unit, mem)
    };
    assert_faults_at_the_last_word(setup, "blr, LR = -1");
}

#[test]
fn ba_with_a_negative_target_faults_the_thread() {
    let setup = || {
        unit_over(I::B {
            offset: -4,
            aa: true,
            link: false,
        })
    };
    assert_faults_at_the_last_word(setup, "ba -4");
}
