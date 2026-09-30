//! The fuzz contract on each decoded instruction and the metamorphic relations it admits.

use cellgov_ps3_abi::hw::spu_isa::SPU_OPCODE_MAP;

use crate::instruction::{SpuInstruction, SpuInstructionKind};
use crate::state::{SpuState, SPU_REG_COUNT};

use super::classify::{classify_kind, effect_and_outcome, form_for_kind};
use super::relations::{
    branch_complement, count_immediate_mask, count_register_bits, ignored_field_mask,
    immediate_register_pair, is_commutative, slot_permutation_reads_rt,
};
use super::support::{
    encoding_execution_is_supported, encoding_has_undefined_operands, execution_supported,
    state_input,
};
use super::types::{
    SpuEncodingForm, SpuFuzzDescriptor, SpuInputRewrite, SpuMetamorphicCase,
    SpuMetamorphicRelation, SpuObservableState, SpuRelationRefusal, SpuVariedInput,
};

use SpuMetamorphicRelation as R;

const RELATIONS: &[SpuMetamorphicRelation] = &[R::Deterministic];
const IGNORED: &[SpuMetamorphicRelation] = &[R::Deterministic, R::IgnoredField];
const IGNORED_SLOTS: &[SpuMetamorphicRelation] =
    &[R::Deterministic, R::IgnoredField, R::SlotPermutation];
const IGNORED_IMMEDIATE: &[SpuMetamorphicRelation] =
    &[R::Deterministic, R::IgnoredField, R::ImmediateRegister];
const IGNORED_COMMUTATIVE: &[SpuMetamorphicRelation] =
    &[R::Deterministic, R::IgnoredField, R::Commutative];
const IGNORED_BRANCH: &[SpuMetamorphicRelation] =
    &[R::Deterministic, R::IgnoredField, R::CompareBranch];
const BRANCH: &[SpuMetamorphicRelation] = &[R::Deterministic, R::CompareBranch];
const IMMEDIATE: &[SpuMetamorphicRelation] = &[R::Deterministic, R::ImmediateRegister];
const IMMEDIATE_SLOTS: &[SpuMetamorphicRelation] =
    &[R::Deterministic, R::ImmediateRegister, R::SlotPermutation];
const COUNT_IMMEDIATE: &[SpuMetamorphicRelation] =
    &[R::Deterministic, R::CountMasking, R::ImmediateRegister];
const COUNT_IMMEDIATE_SLOTS: &[SpuMetamorphicRelation] = &[
    R::Deterministic,
    R::CountMasking,
    R::ImmediateRegister,
    R::SlotPermutation,
];
const COUNT: &[SpuMetamorphicRelation] = &[R::Deterministic, R::CountMasking];
const COUNT_SLOTS: &[SpuMetamorphicRelation] =
    &[R::Deterministic, R::CountMasking, R::SlotPermutation];
const COMMUTATIVE: &[SpuMetamorphicRelation] = &[R::Deterministic, R::Commutative];
const COMMUTATIVE_SLOTS: &[SpuMetamorphicRelation] =
    &[R::Deterministic, R::Commutative, R::SlotPermutation];
const SLOTS: &[SpuMetamorphicRelation] = &[R::Deterministic, R::SlotPermutation];
const SHUFB: &[SpuMetamorphicRelation] = &[R::Deterministic, R::ShufbControlClass];

/// The relations `kind` declares: the deterministic replay, then each
/// relation of the catalog in [`super::relations`] that holds for it.
pub(super) fn relations_for_kind(kind: SpuInstructionKind) -> &'static [SpuMetamorphicRelation] {
    if kind == SpuInstructionKind::Shufb {
        return SHUFB;
    }
    let ignored = ignored_field_mask(kind).is_some();
    let count = count_immediate_mask(kind).is_some() || count_register_bits(kind).is_some();
    let immediate = immediate_register_pair(kind).is_some();
    let commutative = is_commutative(kind);
    let slots = slot_permutation_reads_rt(kind).is_some();
    let branch = branch_complement(kind).is_some();
    match (ignored, count, immediate, commutative, slots, branch) {
        (false, false, false, false, false, false) => RELATIONS,
        (true, false, false, false, false, false) => IGNORED,
        (true, false, false, false, true, false) => IGNORED_SLOTS,
        (true, false, true, false, false, false) => IGNORED_IMMEDIATE,
        (true, false, false, true, false, false) => IGNORED_COMMUTATIVE,
        (true, false, false, false, false, true) => IGNORED_BRANCH,
        (false, false, false, false, false, true) => BRANCH,
        (false, false, true, false, false, false) => IMMEDIATE,
        (false, false, true, false, true, false) => IMMEDIATE_SLOTS,
        (false, true, true, false, false, false) => COUNT_IMMEDIATE,
        (false, true, true, false, true, false) => COUNT_IMMEDIATE_SLOTS,
        (false, true, false, false, false, false) => COUNT,
        (false, true, false, false, true, false) => COUNT_SLOTS,
        (false, false, false, true, false, false) => COMMUTATIVE,
        (false, false, false, true, true, false) => COMMUTATIVE_SLOTS,
        (false, false, false, false, true, false) => SLOTS,
        // A combination with no constant declares no relation beyond the
        // replay; a test checks every kind's set against the catalog.
        _ => RELATIONS,
    }
}

/// Returns another shufb control byte in the same class as `control`.
///
/// A constant pattern keeps its high three bits. A selector keeps its low five.
///
/// [SPU-ISA p:116 s:5 Table 5-1] 10xxxxxx, 110xxxxx and 111xxxxx each give one constant; any other byte selects by its rightmost 5 bits.
fn shufb_class_partner(control: u8) -> u8 {
    if control & 0x80 != 0 {
        control ^ 0x1F
    } else {
        control ^ 0x60
    }
}

/// The word of `kind` with every operand field zero.
pub(super) fn opcode_word(kind: SpuInstructionKind) -> Option<u32> {
    SPU_OPCODE_MAP
        .iter()
        .map(|row| row.canonical_word())
        .find(|&word| {
            crate::decode::decode(word)
                .is_ok_and(|decoded| SpuInstructionKind::from(decoded) == kind)
        })
}

fn rt_field(raw: u32) -> u8 {
    (raw & 0x7F) as u8
}

fn ra_field(raw: u32) -> u8 {
    ((raw >> 7) & 0x7F) as u8
}

fn rb_field(raw: u32) -> u8 {
    ((raw >> 14) & 0x7F) as u8
}

/// A case whose partner is another word run from the original state.
fn word_case(relation: SpuMetamorphicRelation, partner_word: u32) -> SpuMetamorphicCase {
    SpuMetamorphicCase {
        relation,
        partner_word,
        varied_inputs: [None; 3],
        permuted_output: None,
    }
}

/// One varied input; `written` is the register the instruction writes, if any.
fn varied(register: u8, rewrite: SpuInputRewrite, written: Option<u8>) -> SpuVariedInput {
    SpuVariedInput {
        register,
        rewrite,
        restore: Some(register) != written,
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
            relations: relations_for_kind(kind),
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
        let kind = SpuInstructionKind::from(*self);
        if !self.fuzz_descriptor().relations.contains(&relation) || relation == R::Deterministic {
            return Err(SpuRelationRefusal::Undeclared { relation });
        }
        if crate::decode::decode(raw).ok() != Some(*self) {
            return Err(SpuRelationRefusal::InvalidPartner { relation });
        }
        if encoding_has_undefined_operands(raw) || !encoding_execution_is_supported(raw) {
            return Err(SpuRelationRefusal::Ineligible { relation });
        }
        let undeclared = SpuRelationRefusal::Undeclared { relation };
        let no_partner = SpuRelationRefusal::NoPartner { relation };
        let (case, partner_kind) = match relation {
            R::Deterministic => return Err(undeclared),
            R::ShufbControlClass => return self.shufb_control_case(raw, relation),
            R::IgnoredField => {
                let mask = ignored_field_mask(kind).ok_or(undeclared)?;
                (word_case(relation, raw ^ mask), kind)
            }
            R::CountMasking => {
                if let Some(mask) = count_immediate_mask(kind) {
                    (word_case(relation, raw ^ mask), kind)
                } else {
                    let bits = count_register_bits(kind).ok_or(undeclared)?;
                    let (rt, ra, rb) = (rt_field(raw), ra_field(raw), rb_field(raw));
                    // An RB that is also RA is a data input as well as the count.
                    if rb == ra {
                        return Err(no_partner);
                    }
                    let mut case = word_case(relation, raw);
                    case.varied_inputs[0] =
                        Some(varied(rb, SpuInputRewrite::Flip(bits.unread()), Some(rt)));
                    (case, kind)
                }
            }
            R::ImmediateRegister => {
                let (register_kind, splat) = immediate_register_pair(kind).ok_or(undeclared)?;
                let opcode = opcode_word(register_kind).ok_or(undeclared)?;
                let (rt, ra) = (rt_field(raw), ra_field(raw));
                let rb = (0..SPU_REG_COUNT as u8)
                    .find(|&register| register != ra && register != rt)
                    .ok_or(no_partner)?;
                let partner = opcode | u32::from(rb) << 14 | u32::from(ra) << 7 | u32::from(rt);
                let mut case = word_case(relation, partner);
                case.varied_inputs[0] = Some(varied(
                    rb,
                    SpuInputRewrite::Replace(splat.value(raw)),
                    Some(rt),
                ));
                (case, register_kind)
            }
            R::Commutative => {
                let (ra, rb) = (ra_field(raw), rb_field(raw));
                if ra == rb {
                    return Err(no_partner);
                }
                let fields = raw & !0x001F_FF80;
                let partner = fields | u32::from(ra) << 14 | u32::from(rb) << 7;
                (word_case(relation, partner), kind)
            }
            R::SlotPermutation => {
                let reads_rt = slot_permutation_reads_rt(kind).ok_or(undeclared)?;
                let (rt, inputs) = match form_for_kind(kind) {
                    SpuEncodingForm::Rrrr => (
                        ((raw >> 21) & 0x7F) as u8,
                        [
                            Some(ra_field(raw)),
                            Some(rb_field(raw)),
                            Some(rt_field(raw)),
                        ],
                    ),
                    SpuEncodingForm::Rrr => (
                        rt_field(raw),
                        [
                            Some(ra_field(raw)),
                            Some(rb_field(raw)),
                            reads_rt.then_some(rt_field(raw)),
                        ],
                    ),
                    _ => (rt_field(raw), [Some(ra_field(raw)), None, None]),
                };
                let mut case = word_case(relation, raw);
                let mut slot = 0;
                for register in inputs.into_iter().flatten() {
                    let named = case.varied_inputs[..slot]
                        .iter()
                        .flatten()
                        .any(|input| input.register == register);
                    if !named {
                        case.varied_inputs[slot] =
                            Some(varied(register, SpuInputRewrite::SwapDoublewords, Some(rt)));
                        slot += 1;
                    }
                }
                case.permuted_output = Some(rt);
                (case, kind)
            }
            R::CompareBranch => {
                let (partner_kind, halfword) = branch_complement(kind).ok_or(undeclared)?;
                let own = opcode_word(kind).ok_or(undeclared)?;
                let other = opcode_word(partner_kind).ok_or(undeclared)?;
                let rt = rt_field(raw);
                // An indirect branch whose target register is also the tested
                // register would take its target from the rewritten value.
                let indirect = matches!(
                    kind,
                    SpuInstructionKind::Biz
                        | SpuInstructionKind::Binz
                        | SpuInstructionKind::Bihz
                        | SpuInstructionKind::Bihnz
                );
                if indirect && rt == ra_field(raw) {
                    return Err(no_partner);
                }
                let mut case = word_case(relation, raw ^ own ^ other);
                case.varied_inputs[0] =
                    Some(varied(rt, SpuInputRewrite::ZeroCompare { halfword }, None));
                (case, partner_kind)
            }
        };
        if case.partner_word != raw {
            let partner = case.partner_word;
            if encoding_has_undefined_operands(partner) || !encoding_execution_is_supported(partner)
            {
                return Err(SpuRelationRefusal::Ineligible { relation });
            }
            let decoded = crate::decode::decode(partner)
                .map_err(|_| SpuRelationRefusal::InvalidPartner { relation })?;
            if SpuInstructionKind::from(decoded) != partner_kind {
                return Err(SpuRelationRefusal::InvalidPartner { relation });
            }
        }
        Ok(case)
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
        let mut case = word_case(relation, raw);
        case.varied_inputs[0] = Some(varied(rc, SpuInputRewrite::ShufbControlClass, Some(rt)));
        Ok(case)
    }
}

/// `value` with its two doublewords traded.
fn swap_doublewords(value: [u8; 16]) -> [u8; 16] {
    std::array::from_fn(|index| value[(index + 8) % 16])
}

impl SpuInputRewrite {
    /// The partner's value for an input whose original value is `value`.
    fn apply(self, value: [u8; 16]) -> [u8; 16] {
        match self {
            Self::ShufbControlClass => value.map(shufb_class_partner),
            Self::Replace(replacement) => replacement,
            Self::Flip(mask) => std::array::from_fn(|index| value[index] ^ mask[index]),
            Self::ZeroCompare { halfword } => {
                let field = if halfword { 2..4 } else { 0..4 };
                let zero = value[field.clone()].iter().all(|byte| *byte == 0);
                let mut out = value;
                out[field].fill(if zero { 0xFF } else { 0 });
                out
            }
            Self::SwapDoublewords => swap_doublewords(value),
        }
    }
}

impl SpuMetamorphicCase {
    /// The initial state the partner runs from: `initial`, with each varied
    /// input rewritten.
    pub fn partner_initial(&self, initial: &SpuState) -> SpuState {
        let mut partner = initial.clone();
        for input in self.varied_inputs.iter().flatten() {
            let register = usize::from(input.register);
            partner.set_reg(register, input.rewrite.apply(initial.regs[register]));
        }
        partner
    }

    /// Brings `partner_regs` back to the original's frame: swaps the
    /// permuted result back, then copies each varied input the
    /// instruction does not write from `original`, the original run's
    /// final registers.
    ///
    /// The caller runs this before it compares the partner with the
    /// original, so the comparison covers only the relation's claim. A
    /// write to a varied input is the footprint check's finding.
    pub fn settle_partner(
        &self,
        original: &[[u8; 16]; SPU_REG_COUNT],
        partner_regs: &mut [[u8; 16]; SPU_REG_COUNT],
    ) {
        if let Some(output) = self.permuted_output {
            let output = usize::from(output);
            partner_regs[output] = swap_doublewords(partner_regs[output]);
        }
        for input in self.varied_inputs.iter().flatten() {
            if input.restore {
                let register = usize::from(input.register);
                partner_regs[register] = original[register];
            }
        }
    }
}
