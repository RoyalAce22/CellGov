//! Branch dispatch: `b`/`bc`/`bclr`/`bcctr` and the BO/BI condition
//! evaluator they share.
//!
//! Every branch form's pseudocode is (1) optional CTR decrement,
//! (2) condition evaluation, (3) target write, (4) LR write if LK.
//! The dispatch arms below preserve that ordering so adding a side
//! effect to `branch_condition` cannot accidentally observe a
//! half-updated LR.
// [PPC-Book1 p:20 s:2.4] Branch Processor Instructions overview.

use crate::exec::ExecuteVerdict;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;

pub(crate) fn execute(insn: &PpuInstruction, state: &mut PpuState) -> ExecuteVerdict {
    match *insn {
        PpuInstruction::B { offset, aa, link } => {
            // `offset as u64` sign-extends: i32 -> u64 widening from a
            // signed source. This matches `EXTS(LI || 0b00)` so a
            // negative absolute target lands at 0xFFFF_FFFF_FFFF_xxxx.
            // Do not change the cast to `as u32 as u64`; that path
            // would zero-extend.
            // [PPC-Book1 p:24 s:2.4] Branch I-form: NIA <- EXTS(LI||0b00) when AA, else CIA+EXTS(LI||0b00); LR<-CIA+4 when LK.
            let target = if aa {
                (offset as u64) & 0xFFFF_FFFF_FFFF_FFFC
            } else {
                (state.pc as i64).wrapping_add(offset as i64) as u64
            };
            if link {
                state.lr = state.pc + 4;
            }
            state.pc = target;
            ExecuteVerdict::Branch
        }
        PpuInstruction::Bc {
            bo,
            bi,
            offset,
            aa,
            link,
        } => {
            // [PPC-Book1 p:24 s:2.4] Branch Conditional B-form: target is EXTS(BD||0b00); LR is written under LK regardless of taken.
            let cond = branch_condition(state, bo, bi);
            if link {
                state.lr = state.pc + 4;
            }
            if cond {
                state.pc = if aa {
                    (offset as i64 as u64) & 0xFFFF_FFFF_FFFF_FFFC
                } else {
                    (state.pc as i64).wrapping_add(offset as i64) as u64
                };
                ExecuteVerdict::Branch
            } else {
                ExecuteVerdict::Continue
            }
        }
        PpuInstruction::Bclr { bo, bi, link } => {
            // Capture target before LR is overwritten so `blrl`
            // returns to the *old* LR while the new LR points at the
            // following instruction. The architectural BH operand is
            // dropped at decode -- it is a predictor hint only and
            // does not affect results.
            // [PPC-Book1 p:25 s:2.4] bclr XL-form: NIA <- LR[0:61]||0b00; CTR decremented when BO_2=0.
            // [PPC-Book1 p:21 s:2.4.1 Figure 23] BH field encodings; BH is independent of BO "at" hints and does not affect execution.
            let target = state.lr & !3;
            let cond = branch_condition(state, bo, bi);
            if link {
                state.lr = state.pc + 4;
            }
            if cond {
                state.pc = target;
                ExecuteVerdict::Branch
            } else {
                ExecuteVerdict::Continue
            }
        }
        PpuInstruction::Bcctr { bo, bi, link } => {
            // bcctr's pseudocode has no CTR decrement step. BO_2=0
            // is an invalid form (assemblers reject bdnzctr); we
            // treat BO_2 as don't-care here so the invalid form runs
            // deterministically without corrupting CTR. Only the
            // CR-test path matters for the condition.
            // [PPC-Book1 p:25 s:2.4] bcctr XL-form: NIA <- CTR[0:61]||0b00; specifying BO_2=0 yields an invalid form.
            let cond_ok = (bo & 0x10) != 0 || (state.cr_bit(bi) == ((bo & 0x08) != 0));
            if link {
                state.lr = state.pc + 4;
            }
            if cond_ok {
                state.pc = state.ctr & !3;
                ExecuteVerdict::Branch
            } else {
                ExecuteVerdict::Continue
            }
        }
        _ => unreachable!("branch::execute called with non-branch variant"),
    }
}

/// Evaluate a PPC BO/BI branch condition. Decrements CTR as a side
/// effect when BO bit 0x04 is clear.
///
/// BO bits (MSB->LSB): 0x10 skip CR test, 0x08 CR polarity,
/// 0x04 skip CTR decrement, 0x02 CTR-zero polarity, 0x01 hint.
///
/// `bcctr` is the only branch whose spec pseudocode lacks a CTR
/// decrement -- it does not call this helper.
pub(crate) fn branch_condition(state: &mut PpuState, bo: u8, bi: u8) -> bool {
    // [PPC-Book1 p:24 s:2.4] BO encoding: ctr_ok = BO_2 | ((CTR!=0) XOR BO_3); cond_ok = BO_0 | (CR_BI == BO_1).
    // [PPC-Book1 p:20 s:2.4.1 Figure 21] BO field encodings table; the "a"/"t" hint bits in BO_4 (0x01) are software hints only and do not affect results.
    let decr_ctr = (bo & 0x04) == 0;
    if decr_ctr {
        state.ctr = state.ctr.wrapping_sub(1);
    }

    let ctr_ok = (bo & 0x04) != 0 || ((state.ctr != 0) ^ ((bo & 0x02) != 0));
    let cr_ok = (bo & 0x10) != 0 || (state.cr_bit(bi) == ((bo & 0x08) != 0));

    ctr_ok && cr_ok
}

#[cfg(test)]
#[path = "tests/branch_tests.rs"]
mod tests;
