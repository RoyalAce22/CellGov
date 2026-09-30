//! The fuzz contract on each decoded instruction and the metamorphic relations it admits.

use crate::instruction::{SpuInstruction, SpuInstructionKind};
use crate::state::{SpuState, SPU_REG_COUNT};

use super::classify::{classify_kind, effect_and_outcome, form_for_kind};
use super::support::{
    encoding_execution_is_supported, encoding_has_undefined_operands, execution_supported,
    state_input,
};
use super::types::{
    SpuFuzzDescriptor, SpuMetamorphicCase, SpuMetamorphicRelation, SpuObservableState,
    SpuRelationRefusal, SpuVariedInput,
};

const NOP_RELATIONS: &[SpuMetamorphicRelation] = &[
    SpuMetamorphicRelation::Deterministic,
    SpuMetamorphicRelation::NopFalseTarget,
];
const ROTATE_RELATIONS: &[SpuMetamorphicRelation] = &[
    SpuMetamorphicRelation::Deterministic,
    SpuMetamorphicRelation::RotateByteCountHighBit,
];
const SHUFB_RELATIONS: &[SpuMetamorphicRelation] = &[
    SpuMetamorphicRelation::Deterministic,
    SpuMetamorphicRelation::ShufbControlClass,
];
const RELATIONS: &[SpuMetamorphicRelation] = &[SpuMetamorphicRelation::Deterministic];

/// Returns another shufb control byte in the same class as `control`.
///
/// A constant pattern keeps its high three bits. A selector keeps its low five.
// [SPU-ISA p:116 s:5 Table 5-1] 10xxxxxx, 110xxxxx and 111xxxxx each give one constant; any other byte selects by its rightmost 5 bits.
fn shufb_class_partner(control: u8) -> u8 {
    if control & 0x80 != 0 {
        control ^ 0x1F
    } else {
        control ^ 0x60
    }
}

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
            relations: match kind {
                SpuInstructionKind::Rotqbyi => ROTATE_RELATIONS,
                SpuInstructionKind::Nop => NOP_RELATIONS,
                SpuInstructionKind::Shufb => SHUFB_RELATIONS,
                _ => RELATIONS,
            },
            decoded_execution_supported: execution_supported(*self),
            state_input: state_input(*self),
        }
    }

    /// Derives a same-observation partner from an eligible instruction.
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
            SpuMetamorphicRelation::ShufbControlClass => {
                return self.shufb_control_case(raw, relation);
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
            varied_input: None,
        })
    }

    /// The shufb control-class case: the same word, with RC rewritten.
    ///
    /// An RC that aliases RA or RB is also a data input, and a rewrite of it
    /// can change a selected byte. Such an encoding has no partner.
    fn shufb_control_case(
        &self,
        raw: u32,
        relation: SpuMetamorphicRelation,
    ) -> Result<SpuMetamorphicCase, SpuRelationRefusal> {
        let SpuInstruction::Shufb { rt, ra, rb, rc } = *self else {
            return Err(SpuRelationRefusal::Undeclared { relation });
        };
        if rc == ra || rc == rb {
            return Err(SpuRelationRefusal::NoPartner { relation });
        }
        Ok(SpuMetamorphicCase {
            relation,
            partner_word: raw,
            varied_input: Some(SpuVariedInput {
                register: rc,
                restore: rc != rt,
            }),
        })
    }
}

impl SpuMetamorphicCase {
    /// The initial state the partner runs from: `initial`, with the varied
    /// input rewritten when the relation has one.
    pub fn partner_initial(&self, initial: &SpuState) -> SpuState {
        let mut partner = initial.clone();
        if let (SpuMetamorphicRelation::ShufbControlClass, Some(input)) =
            (self.relation, self.varied_input)
        {
            let register = &mut partner.regs[usize::from(input.register)];
            *register = register.map(shufb_class_partner);
        }
        partner
    }

    /// Restores the varied input in `partner_regs` to its value in `initial`.
    ///
    /// The restore applies only when the instruction does not write that
    /// register. The caller runs this before it compares the partner with
    /// the original, so the comparison covers only the relation's claim.
    pub fn settle_partner(&self, initial: &SpuState, partner_regs: &mut [[u8; 16]; SPU_REG_COUNT]) {
        if let Some(SpuVariedInput {
            register,
            restore: true,
        }) = self.varied_input
        {
            let register = usize::from(register);
            partner_regs[register] = initial.regs[register];
        }
    }
}
