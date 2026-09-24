//! The fuzz contract on each decoded instruction and the metamorphic relations it admits.

use crate::instruction::ops::Fp63Op;
use crate::instruction::{PpuInstruction, PpuInstructionKind};
use crate::state::PpuState;

use super::classify::{classify_kind, effect_and_outcome, form_for_word, fuzz_kind};
use super::fields::generation_operands_are_valid;
use super::types::{
    PpuFuzzDescriptor, PpuMetamorphicCase, PpuMetamorphicRelation, PpuObservableState,
    PpuPermittedDelta, PpuRelationRefusal,
};

const RELATIONS: &[PpuMetamorphicRelation] = &[PpuMetamorphicRelation::Deterministic];
const RECORD_CR0_RELATIONS: &[PpuMetamorphicRelation] = &[
    PpuMetamorphicRelation::Deterministic,
    PpuMetamorphicRelation::RecordCr0,
];
const RECORD_CR1_RELATIONS: &[PpuMetamorphicRelation] = &[
    PpuMetamorphicRelation::Deterministic,
    PpuMetamorphicRelation::RecordCr1,
];
const RECORD_CR6_RELATIONS: &[PpuMetamorphicRelation] = &[
    PpuMetamorphicRelation::Deterministic,
    PpuMetamorphicRelation::RecordCr6,
];
const RECORD_CR0_OE_RELATIONS: &[PpuMetamorphicRelation] = &[
    PpuMetamorphicRelation::Deterministic,
    PpuMetamorphicRelation::RecordCr0,
    PpuMetamorphicRelation::OverflowEnable,
];

impl PpuInstruction {
    /// Return the interpreter-owned fuzz contract for this instruction.
    pub fn fuzz_descriptor(&self, raw: u32) -> PpuFuzzDescriptor {
        let instruction_kind = PpuInstructionKind::from(*self);
        classify_kind(instruction_kind);
        let (effects, outcomes) =
            effect_and_outcome(instruction_kind, generation_operands_are_valid(*self));
        PpuFuzzDescriptor {
            kind: fuzz_kind(*self),
            form: form_for_word(instruction_kind, raw),
            observable_state: PpuObservableState::Complete,
            effects,
            outcomes,
            relations: relations_for_instruction(*self),
        }
    }

    /// Builds an eligible partner for one declared relation.
    // [Le2014 p:219 s:3.1.1] The partner is equivalent to the original only over inputs on which both are defined.
    pub fn metamorphic_case(
        &self,
        raw: u32,
        state: &PpuState,
        relation: PpuMetamorphicRelation,
    ) -> Result<PpuMetamorphicCase, PpuRelationRefusal> {
        if !self.fuzz_descriptor(raw).relations.contains(&relation) {
            return Err(PpuRelationRefusal::Undeclared { relation });
        }
        if self.fuzz_case_is_architecturally_undefined(state) {
            return Err(PpuRelationRefusal::ArchitecturallyUndefined { relation });
        }
        let (mask, permitted_delta) = match relation {
            PpuMetamorphicRelation::Deterministic => {
                return Err(PpuRelationRefusal::Undeclared { relation });
            }
            PpuMetamorphicRelation::RecordCr0 => (0x0000_0001, PpuPermittedDelta::Cr0),
            PpuMetamorphicRelation::RecordCr1 => (0x0000_0001, PpuPermittedDelta::Cr1),
            PpuMetamorphicRelation::RecordCr6 => (0x0000_0400, PpuPermittedDelta::Cr6),
            PpuMetamorphicRelation::OverflowEnable => (0x0000_0400, PpuPermittedDelta::XerOverflow),
        };
        if raw & mask != 0 {
            return Err(PpuRelationRefusal::AlreadyEnabled { relation });
        }
        if relation == PpuMetamorphicRelation::OverflowEnable && raw & 1 != 0 {
            return Err(PpuRelationRefusal::IncompatibleControls { relation });
        }
        let partner_word = raw | mask;
        let partner = crate::decode::decode(partner_word)
            .map_err(|_| PpuRelationRefusal::InvalidPartner { relation })?;
        if partner.fuzz_descriptor(partner_word).kind != self.fuzz_descriptor(raw).kind {
            return Err(PpuRelationRefusal::InvalidPartner { relation });
        }
        Ok(PpuMetamorphicCase {
            relation,
            partner_word,
            permitted_delta,
        })
    }

    /// Classifies instruction-state pairs that fuzz comparison must exclude.
    pub fn fuzz_case_is_architecturally_undefined(&self, state: &PpuState) -> bool {
        use PpuInstruction as I;

        match *self {
            // [PPC-Book1 p:58 s:3.3.8] Word division leaves half of RT undefined.
            I::Divw { .. } | I::Divwu { .. } => true,
            // [PPC-Book1 p:58 s:3.3.8] Signed doubleword division is undefined for zero and MIN/-1.
            I::Divd { ra, rb, .. } => {
                let dividend = state.gpr[ra as usize] as i64;
                let divisor = state.gpr[rb as usize] as i64;
                divisor == 0 || (dividend == i64::MIN && divisor == -1)
            }
            // [PPC-Book1 p:59 s:3.3.8] Unsigned doubleword division is undefined for zero.
            I::Divdu { rb, .. } => state.gpr[rb as usize] == 0,
            // [PPC-Book1 p:117 s:4.6.6] fctiw and fctiwz leave the high half of FRT undefined.
            I::Fp63 {
                op: Fp63Op::Fctiw | Fp63Op::Fctiwz,
                ..
            } => true,
            // [PPC-Book1 p:120 s:4.6.8] mffs leaves the high half of FRT undefined.
            I::Fp63 {
                op: Fp63Op::Mffs, ..
            } => true,
            // [PowerISA-3.1 p:I128 s:3.3] mfocrf leaves RT undefined unless FXM is one-hot.
            I::Mfocrf { crm, .. } => crm.count_ones() != 1,
            // [PPC-Book1 p:124 s:5.1.1] mtocrf leaves CR undefined unless FXM is one-hot.
            I::Mtocrf { crm, .. } => crm.count_ones() != 1,
            // [PPC-Book1 p:48 s:3.3] lswx leaves RT undefined when the byte count is zero.
            I::Lswx { .. } => state.xer_tbc() == 0,
            _ => false,
        }
    }
}

fn relations_for_instruction(instruction: PpuInstruction) -> &'static [PpuMetamorphicRelation] {
    use PpuInstruction as I;

    match instruction {
        I::Add { .. }
        | I::Subf { .. }
        | I::Subfc { .. }
        | I::Subfe { .. }
        | I::Neg { .. }
        | I::Mullw { .. }
        | I::Adde { .. }
        | I::Addze { .. }
        | I::Subfze { .. }
        | I::Subfme { .. }
        | I::Addme { .. }
        | I::Mulld { .. }
        | I::Divw { .. }
        | I::Divwu { .. }
        | I::Divd { .. }
        | I::Divdu { .. } => RECORD_CR0_OE_RELATIONS,
        I::Or { .. }
        | I::Mulhwu { .. }
        | I::Mulhw { .. }
        | I::Mulhdu { .. }
        | I::Mulhd { .. }
        | I::And { .. }
        | I::Andc { .. }
        | I::Nor { .. }
        | I::Xor { .. }
        | I::Eqv { .. }
        | I::Nand { .. }
        | I::Orc { .. }
        | I::Slw { .. }
        | I::Srw { .. }
        | I::Srd { .. }
        | I::Srawi { .. }
        | I::Sraw { .. }
        | I::Srad { .. }
        | I::Sradi { .. }
        | I::Sld { .. }
        | I::Cntlzw { .. }
        | I::Cntlzd { .. }
        | I::Extsh { .. }
        | I::Extsb { .. }
        | I::Extsw { .. }
        | I::Rlwinm { .. }
        | I::Rlwimi { .. }
        | I::Rlwnm { .. }
        | I::Rldicl { .. }
        | I::Rldicr { .. }
        | I::Rldic { .. }
        | I::Rldimi { .. }
        | I::Rldcl { .. }
        | I::Rldcr { .. } => RECORD_CR0_RELATIONS,
        I::Fp59 { .. }
        | I::Fp63 {
            op:
                Fp63Op::Frsp
                | Fp63Op::Fctiw
                | Fp63Op::Fctiwz
                | Fp63Op::Fdiv
                | Fp63Op::Fsub
                | Fp63Op::Fadd
                | Fp63Op::Fsqrt
                | Fp63Op::Fsel
                | Fp63Op::Fmul
                | Fp63Op::Frsqrte
                | Fp63Op::Fmsub
                | Fp63Op::Fmadd
                | Fp63Op::Fnmsub
                | Fp63Op::Fnmadd
                | Fp63Op::Mtfsb1
                | Fp63Op::Fneg
                | Fp63Op::Mtfsb0
                | Fp63Op::Fmr
                | Fp63Op::Mtfsfi
                | Fp63Op::Fnabs
                | Fp63Op::Fabs
                | Fp63Op::Mffs
                | Fp63Op::Mtfsf
                | Fp63Op::Fctid
                | Fp63Op::Fctidz
                | Fp63Op::Fcfid,
            ..
        } => RECORD_CR1_RELATIONS,
        // [PPC-Book1 p:119 s:4.6.7] Floating compares reserve raw bit 31.
        // [PPC-Book1 p:120 s:4.6.8] mcrfs also reserves raw bit 31.
        I::Fp63 {
            op: Fp63Op::Fcmpu | Fp63Op::Fcmpo | Fp63Op::Mcrfs,
            ..
        } => RELATIONS,
        I::Vx { op, .. } if op.is_vxr_compare() => RECORD_CR6_RELATIONS,
        _ => RELATIONS,
    }
}
