//! CR-logical XL-form dispatch: `mcrf` plus the eight one-bit
//! Boolean ops on CR bits (`crand`, `crandc`, `cror`, `crorc`,
//! `crxor`, `crnand`, `crnor`, `creqv`).
// [PPC-Book1 p:28 s:2.4.3] CR-logical XL-form: crand/cror/crxor/crnand.
// [PPC-Book1 p:29 s:2.4.3] CR-logical XL-form: crnor/creqv/crandc/crorc.
// [PPC-Book1 p:30 s:2.4.4] mcrf XL-form: copy CR field BFA into BF.
//!
//! Each handler reads up to two source CR bits, computes the
//! Boolean op, writes one destination CR bit, and returns
//! `ExecuteVerdict::Continue`. PC, LR, CTR are untouched.

use crate::exec::ExecuteVerdict;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;

pub(crate) fn execute(insn: &PpuInstruction, state: &mut PpuState) -> ExecuteVerdict {
    match *insn {
        PpuInstruction::Mcrf { crfd, crfs } => {
            // [PPC-Book1 p:30 s:2.4.4] mcrf BF,BFA copies CR field BFA into BF.
            let val = state.cr_field(crfs);
            state.set_cr_field(crfd, val);
        }
        PpuInstruction::Crand { bt, ba, bb } => {
            // [PPC-Book1 p:28 s:2.4.3] crand: CR[BT] = CR[BA] & CR[BB].
            state.set_cr_bit(bt, state.cr_bit(ba) && state.cr_bit(bb));
        }
        PpuInstruction::Crandc { bt, ba, bb } => {
            // [PPC-Book1 p:29 s:2.4.3] crandc: CR[BT] = CR[BA] & !CR[BB].
            state.set_cr_bit(bt, state.cr_bit(ba) && !state.cr_bit(bb));
        }
        PpuInstruction::Cror { bt, ba, bb } => {
            // [PPC-Book1 p:28 s:2.4.3] cror: CR[BT] = CR[BA] | CR[BB].
            state.set_cr_bit(bt, state.cr_bit(ba) || state.cr_bit(bb));
        }
        PpuInstruction::Crorc { bt, ba, bb } => {
            // [PPC-Book1 p:29 s:2.4.3] crorc: CR[BT] = CR[BA] | !CR[BB].
            state.set_cr_bit(bt, state.cr_bit(ba) || !state.cr_bit(bb));
        }
        PpuInstruction::Crxor { bt, ba, bb } => {
            // [PPC-Book1 p:28 s:2.4.3] crxor: CR[BT] = CR[BA] ^ CR[BB].
            state.set_cr_bit(bt, state.cr_bit(ba) ^ state.cr_bit(bb));
        }
        PpuInstruction::Crnand { bt, ba, bb } => {
            // [PPC-Book1 p:28 s:2.4.3] crnand: CR[BT] = !(CR[BA] & CR[BB]).
            state.set_cr_bit(bt, !(state.cr_bit(ba) && state.cr_bit(bb)));
        }
        PpuInstruction::Crnor { bt, ba, bb } => {
            // [PPC-Book1 p:29 s:2.4.3] crnor: CR[BT] = !(CR[BA] | CR[BB]).
            state.set_cr_bit(bt, !(state.cr_bit(ba) || state.cr_bit(bb)));
        }
        PpuInstruction::Creqv { bt, ba, bb } => {
            // [PPC-Book1 p:29 s:2.4.3] creqv: CR[BT] = !(CR[BA] ^ CR[BB]).
            state.set_cr_bit(bt, !(state.cr_bit(ba) ^ state.cr_bit(bb)));
        }
        _ => unreachable!("cr::execute called with non-CR-logical variant"),
    }
    ExecuteVerdict::Continue
}

#[cfg(test)]
#[path = "tests/cr_tests.rs"]
mod tests;
