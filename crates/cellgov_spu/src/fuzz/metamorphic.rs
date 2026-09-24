//! The fuzz contract on each decoded instruction and the metamorphic relations it admits.

use crate::instruction::{SpuInstruction, SpuInstructionKind};

use super::classify::{classify_kind, effect_and_outcome, form_for_kind};
use super::support::{
    encoding_execution_is_supported, encoding_has_undefined_operands, execution_supported,
    state_input,
};
use super::types::{
    SpuFuzzDescriptor, SpuMetamorphicCase, SpuMetamorphicRelation, SpuObservableState,
    SpuRelationRefusal,
};

const NOP_RELATIONS: &[SpuMetamorphicRelation] = &[
    SpuMetamorphicRelation::Deterministic,
    SpuMetamorphicRelation::NopFalseTarget,
];
const ROTATE_RELATIONS: &[SpuMetamorphicRelation] = &[
    SpuMetamorphicRelation::Deterministic,
    SpuMetamorphicRelation::RotateByteCountHighBit,
];
const RELATIONS: &[SpuMetamorphicRelation] = &[SpuMetamorphicRelation::Deterministic];

impl SpuInstruction {
    /// Return the interpreter-owned fuzz contract for this instruction.
    pub fn fuzz_descriptor(&self) -> SpuFuzzDescriptor {
        let kind = SpuInstructionKind::from(*self);
        classify_kind(kind);
        let (effects, outcomes) = effect_and_outcome(self);
        SpuFuzzDescriptor {
            kind,
            form: form_for_kind(kind),
            observable_state: SpuObservableState::Complete,
            effects,
            outcomes,
            relations: if kind == SpuInstructionKind::Rotqbyi {
                ROTATE_RELATIONS
            } else if kind == SpuInstructionKind::Nop {
                NOP_RELATIONS
            } else {
                RELATIONS
            },
            decoded_execution_supported: execution_supported(*self),
            state_input: state_input(*self),
        }
    }

    /// Derives a same-observation partner word from an eligible instruction.
    ///
    /// # Errors
    ///
    /// Returns a typed refusal if the transformation is inapplicable or unsafe.
    pub fn metamorphic_case(
        &self,
        raw: u32,
        relation: SpuMetamorphicRelation,
    ) -> Result<SpuMetamorphicCase, SpuRelationRefusal> {
        // [Le2014 p:219 s:3.1.1] The partner is equivalent to the original only over inputs on which both are defined.
        if !self.fuzz_descriptor().relations.contains(&relation)
            || relation == SpuMetamorphicRelation::Deterministic
        {
            return Err(SpuRelationRefusal::Undeclared { relation });
        }
        if crate::decode::decode(raw).ok() != Some(*self) {
            return Err(SpuRelationRefusal::InvalidPartner { relation });
        }
        if encoding_has_undefined_operands(raw) || !encoding_execution_is_supported(raw) {
            return Err(SpuRelationRefusal::Ineligible { relation });
        }
        let partner_word = match relation {
            SpuMetamorphicRelation::NopFalseTarget => {
                // [SPU-ISA p:241 s:10 Control Instructions] NOP does not use its RT false target.
                raw ^ 1
            }
            SpuMetamorphicRelation::RotateByteCountHighBit => {
                // [SPU-ISA p:132 s:6. Shift and Rotate Instructions] ROTQBYI uses only I7's low four bits for its byte count.
                raw ^ 0x0004_0000
            }
            SpuMetamorphicRelation::Deterministic => {
                return Err(SpuRelationRefusal::Undeclared { relation })
            }
        };
        if encoding_has_undefined_operands(partner_word)
            || !encoding_execution_is_supported(partner_word)
        {
            return Err(SpuRelationRefusal::Ineligible { relation });
        }
        let same_decoding = crate::decode::decode(partner_word).ok() == Some(*self);
        let same_rotation = matches!((relation, *self, crate::decode::decode(partner_word)),
            (SpuMetamorphicRelation::RotateByteCountHighBit,
             SpuInstruction::Rotqbyi { rt, ra, imm },
             Ok(SpuInstruction::Rotqbyi { rt: other_rt, ra: other_ra, imm: other_imm }))
            if rt == other_rt && ra == other_ra && (imm & 0x0f) == (other_imm & 0x0f));
        if !(same_decoding && relation == SpuMetamorphicRelation::NopFalseTarget || same_rotation) {
            return Err(SpuRelationRefusal::InvalidPartner { relation });
        }
        Ok(SpuMetamorphicCase {
            relation,
            partner_word,
        })
    }
}
