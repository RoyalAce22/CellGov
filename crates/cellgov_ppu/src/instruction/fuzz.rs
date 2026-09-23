//! Defines interpreter-owned PPU instruction contracts for fuzzers.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_effects::EffectKind;
use strum::{IntoEnumIterator, VariantArray};

use super::ops::{Fp59Op, Fp59Shape, Fp63Op, Fp63Shape, VaOp, VaShape, VxOp, VxShape};
use super::{PpuInstruction, PpuInstructionKind};
use crate::state::PpuState;

/// Encoding form used by a decoded PPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuEncodingForm {
    /// PowerPC D-form encoding.
    D,
    /// PowerPC DS-form encoding.
    Ds,
    /// PowerPC I-form encoding.
    I,
    /// PowerPC B-form encoding.
    B,
    /// PowerPC X or XO-form encoding.
    X,
    /// PowerPC XL-form encoding.
    Xl,
    /// PowerPC M-form encoding.
    M,
    /// PowerPC MD or MDS-form encoding.
    Md,
    /// AltiVec VA or VX-form encoding.
    Vector,
    /// Floating-point A or X-form encoding.
    Float,
    /// System-call encoding.
    SystemCall,
    /// A predecoded synthetic instruction with no standalone encoding.
    Synthetic,
}

/// Exact fuzz identity for a decoded PPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuFuzzKind {
    /// An instruction represented by its ordinary typed variant.
    Ordinary(PpuInstructionKind),
    /// One operation in the generic VX family.
    Vx(VxOp),
    /// One operation in the generic VA family.
    Va(VaOp),
    /// One operation in the generic primary-59 floating-point family.
    Fp59(Fp59Op),
    /// One operation in the generic primary-63 floating-point family.
    Fp63(Fp63Op),
}

impl PartialOrd for PpuFuzzKind {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PpuFuzzKind {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        fuzz_kind_order(*self).cmp(&fuzz_kind_order(*other))
    }
}

fn fuzz_kind_order(kind: PpuFuzzKind) -> (u8, u16) {
    match kind {
        PpuFuzzKind::Ordinary(kind) => (0, kind as u16),
        PpuFuzzKind::Vx(op) => (1, op as u16),
        PpuFuzzKind::Va(op) => (2, op as u16),
        PpuFuzzKind::Fp59(op) => (3, op as u16),
        PpuFuzzKind::Fp63(op) => (4, op as u16),
    }
}

/// Observable state compared after PPU execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuObservableState {
    /// GPR, FPR, VR, CR, LR, CTR, XER, PC, reservation, memory, and effects.
    Complete,
}

/// Legal result class for one PPU instruction step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuOutcomeClass {
    /// Ordinary completion.
    Continue,
    /// Explicit PC change.
    Branch,
    /// System-call boundary.
    Syscall,
    /// Architectural fault.
    Fault,
    /// Unmapped-memory fault.
    MemoryFault,
    /// Caller must flush the store buffer and retry.
    BufferFull,
}

impl PpuOutcomeClass {
    /// Classify an executor verdict exhaustively.
    pub fn from_verdict(verdict: &crate::exec::ExecuteVerdict) -> Self {
        match verdict {
            crate::exec::ExecuteVerdict::Continue => Self::Continue,
            crate::exec::ExecuteVerdict::Branch => Self::Branch,
            crate::exec::ExecuteVerdict::Syscall { .. } => Self::Syscall,
            crate::exec::ExecuteVerdict::Fault(_) => Self::Fault,
            crate::exec::ExecuteVerdict::MemFault(_) => Self::MemoryFault,
            crate::exec::ExecuteVerdict::BufferFull => Self::BufferFull,
        }
    }
}

/// Interpreter self-relation suitable for fuzz checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuMetamorphicRelation {
    /// Identical inputs give identical outputs.
    Deterministic,
    /// Rc permits changes only to CR field 0.
    RecordCr0,
    /// Rc permits changes only to CR field 1.
    RecordCr1,
    /// Rc on a vector compare permits changes only to CR field 6.
    RecordCr6,
    /// OE permits changes only to the XER overflow fields.
    OverflowEnable,
}

/// Observation field that one metamorphic relation may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuPermittedDelta {
    /// Condition-register field 0.
    Cr0,
    /// Condition-register field 1.
    Cr1,
    /// Condition-register field 6.
    Cr6,
    /// XER overflow and summary-overflow bits.
    XerOverflow,
}

/// One eligible transformed input and its comparison rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuMetamorphicCase {
    /// Relation that produced this case.
    pub relation: PpuMetamorphicRelation,
    /// Transformed instruction word.
    pub partner_word: u32,
    /// Architectural delta the relation permits.
    pub permitted_delta: PpuPermittedDelta,
}

/// Why a requested metamorphic partner is inapplicable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PpuRelationRefusal {
    /// The instruction does not declare this relation.
    #[error("PPU instruction does not declare relation {relation:?}")]
    Undeclared {
        /// Requested relation.
        relation: PpuMetamorphicRelation,
    },
    /// The transformed control is already enabled.
    #[error("PPU relation {relation:?} requires its control bit to be clear")]
    AlreadyEnabled {
        /// Requested relation.
        relation: PpuMetamorphicRelation,
    },
    /// Another enabled control would widen the permitted observation delta.
    #[error("PPU relation {relation:?} requires other controls to be clear")]
    IncompatibleControls {
        /// Requested relation.
        relation: PpuMetamorphicRelation,
    },
    /// The instruction-state pair has undefined architectural behavior.
    #[error("PPU relation {relation:?} is undefined for this input state")]
    ArchitecturallyUndefined {
        /// Requested relation.
        relation: PpuMetamorphicRelation,
    },
    /// The transformed word did not preserve the exact instruction kind.
    #[error("PPU relation {relation:?} produced an invalid partner")]
    InvalidPartner {
        /// Requested relation.
        relation: PpuMetamorphicRelation,
    },
}

/// Complete fuzz contract for one decoded PPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuFuzzDescriptor {
    /// Stable instruction identity.
    pub kind: PpuFuzzKind,
    /// Encoding form of the decoded instruction.
    pub form: PpuEncodingForm,
    /// State projection used for comparison.
    pub observable_state: PpuObservableState,
    /// Effect variants this instruction may emit.
    pub effects: &'static [EffectKind],
    /// Result classes accepted from execution.
    pub outcomes: &'static [PpuOutcomeClass],
    /// Relations that apply to this instruction.
    pub relations: &'static [PpuMetamorphicRelation],
}

/// Semantic class of one encoded PPU operand field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PpuOperandClass {
    /// Selects an architected register.
    Register,
    /// Carries an immediate operand.
    Immediate,
    /// Selects an architected condition field.
    Condition,
    /// Carries a one-bit encoding option.
    Flag,
    /// Selects an architected non-register value.
    Selector,
}

/// Sequence-level interaction with the XER byte-count field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuSequenceClass {
    /// The instruction does not constrain the generated sequence's XER policy.
    Independent,
    /// The instruction reads the XER byte count during execution.
    ReadsXerByteCount,
    /// The instruction replaces XER and can invalidate a later byte-count reader.
    ReplacesXer,
}

/// Control-flow behavior relevant to bounded sequence generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuSequenceFlow {
    /// Execution normally advances to the next generated word.
    Linear,
    /// Execution depends on architectural state that earlier words can replace.
    StateDependent,
    /// Execution can select another program counter.
    ControlTransfer,
    /// Execution ends the generated sequence at this instruction.
    Terminal,
}

/// Register dependency a descriptor can preserve across generated words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuSequenceDependency {
    /// The instruction reads and replaces one general-purpose register.
    GeneralPurposeRegister,
}

/// One packed operand field in a PPU encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuOperandField {
    /// Semantic operand class.
    pub class: PpuOperandClass,
    /// Bits occupied by the field in the instruction word.
    pub mask: u32,
}

impl PpuOperandField {
    /// Returns the largest unpacked value this field accepts.
    pub fn maximum(self) -> u32 {
        low_mask(self.mask.count_ones())
    }

    /// Returns values at important signed and unsigned boundaries.
    pub fn boundary_values(self) -> Vec<u32> {
        let maximum = self.maximum();
        let sign = 1u32
            .checked_shl(self.mask.count_ones().saturating_sub(1))
            .unwrap_or(0);
        let mut values = vec![0, 1.min(maximum), sign.saturating_sub(1), sign, maximum];
        values.sort_unstable();
        values.dedup();
        values
    }
}

/// Interpreter-owned recipe for constructing one exact PPU instruction kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PpuGenerationDescriptor {
    /// Exact instruction kind generated by this recipe.
    pub kind: PpuFuzzKind,
    /// Encoding form.
    pub form: PpuEncodingForm,
    /// Sequence-level state interaction.
    pub sequence_class: PpuSequenceClass,
    /// Governs where generation may place this instruction.
    pub sequence_flow: PpuSequenceFlow,
    /// Marks a register dependency that generation can preserve.
    pub sequence_dependency: Option<PpuSequenceDependency>,
    /// Known decodable word used when every operand is at its canonical value.
    pub canonical_word: u32,
    /// Typed operand fields in low-to-high bit order.
    pub operands: Vec<PpuOperandField>,
}

/// Reports a structural PPU encoding request that conflicts with its descriptor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PpuGenerationError {
    /// The parameter stream did not contain one value per operand field.
    #[error("PPU generation expected {expected} operands but received {found}")]
    OperandCount {
        /// Operand count declared by the descriptor.
        expected: usize,
        /// Operand count supplied by the caller.
        found: usize,
    },
    /// The values do not encode a valid form of the selected exact kind.
    #[error("PPU operands are invalid for selected instruction kind")]
    InvalidOperands,
}

impl PpuGenerationDescriptor {
    /// Encodes one value per typed operand field.
    ///
    /// # Errors
    ///
    /// - If the number of values is incorrect, the method returns [`PpuGenerationError::OperandCount`].
    /// - If the operands are invalid, the method returns [`PpuGenerationError::InvalidOperands`].
    pub fn encode(&self, values: &[u32]) -> Result<u32, PpuGenerationError> {
        let word = self.pack_operands(values)?;
        let instruction =
            crate::decode::decode(word).map_err(|_| PpuGenerationError::InvalidOperands)?;
        (instruction.fuzz_descriptor(word).kind == self.kind
            && generation_operands_are_valid(instruction))
        .then_some(word)
        .ok_or(PpuGenerationError::InvalidOperands)
    }

    /// Packs in-range descriptor operands without consulting the decoder.
    pub fn pack_operands(&self, values: &[u32]) -> Result<u32, PpuGenerationError> {
        if values.len() != self.operands.len() {
            return Err(PpuGenerationError::OperandCount {
                expected: self.operands.len(),
                found: values.len(),
            });
        }
        if values
            .iter()
            .zip(&self.operands)
            .any(|(value, field)| *value > field.maximum())
        {
            return Err(PpuGenerationError::InvalidOperands);
        }
        let mut word = self.canonical_word;
        for (field, value) in self.operands.iter().zip(values) {
            word = (word & !field.mask) | deposit_bits(*value, field.mask);
        }
        Ok(word)
    }

    /// Checks documented operand restrictions using raw fields, independent of decoding.
    pub fn operands_are_defined(&self, word: u32) -> bool {
        use PpuInstructionKind as K;
        let rt = (word >> 21) & 31;
        let ra = (word >> 16) & 31;
        let rb = (word >> 11) & 31;
        match self.kind {
            // [PPC-Book1 p:33 s:3.3.2] Fixed-point load-update requires a nonzero RA distinct from RT.
            PpuFuzzKind::Ordinary(
                K::Lhau
                | K::Lwzu
                | K::Lbzu
                | K::Lhzu
                | K::Ldu
                | K::Lwzux
                | K::Lbzux
                | K::Lhzux
                | K::Ldux
                | K::Lhaux
                | K::Lwaux,
            ) => ra != 0 && ra != rt,
            // [PPC-Book1 p:46 s:3.3.5] LMW may not overwrite its address register.
            PpuFuzzKind::Ordinary(K::Lmw) => ra != 0 && ra < rt,
            // [PPC-Book1 p:48 s:3.3.6] LSWI may not overwrite its address register.
            PpuFuzzKind::Ordinary(K::Lswi) => {
                let nb = if rb == 0 { 32 } else { rb };
                ra != 0 && !(0..nb.div_ceil(4)).any(|index| (rt + index) & 31 == ra)
            }
            // [PPC-Book1 p:48 s:3.3.6] LSWX cannot overwrite either address register first.
            PpuFuzzKind::Ordinary(K::Lswx) => rt != ra && rt != rb,
            // [PPC-Book1 p:104 s:4.6.2] Floating-point load-update requires nonzero RA.
            PpuFuzzKind::Ordinary(K::Lfsu | K::Lfdu | K::Lfsux | K::Lfdux) => ra != 0,
            // [PPC-Book1 p:40 s:3.3.3] Fixed-point store-update requires nonzero RA.
            PpuFuzzKind::Ordinary(
                K::Stwu | K::Stdu | K::Stbu | K::Sthu | K::Stdux | K::Sthux | K::Stwux | K::Stbux,
            ) => ra != 0,
            // [PPC-Book1 p:107 s:4.6] Floating-point store-update requires nonzero RA.
            PpuFuzzKind::Ordinary(K::Stfsu | K::Stfdu | K::Stfsux | K::Stfdux) => ra != 0,
            // [PPC-Book1 p:25 s:2.4] BCCTR cannot request CTR decrement.
            PpuFuzzKind::Ordinary(K::Bcctr) => rt & 0x04 != 0,
            _ => true,
        }
    }

    /// Reads the canonical values in operand order.
    pub fn canonical_parameters(&self) -> Vec<u32> {
        self.operands
            .iter()
            .map(|field| extract_bits(self.canonical_word, field.mask))
            .collect()
    }

    /// Produces one valid witness for each immediate boundary.
    pub fn immediate_boundary_words(&self) -> Vec<u32> {
        let canonical = self.canonical_parameters();
        let mut words = BTreeSet::new();
        for (index, field) in self.operands.iter().enumerate() {
            if field.class != PpuOperandClass::Immediate {
                continue;
            }
            for value in field.boundary_values() {
                let mut parameters = canonical.clone();
                parameters[index] = value;
                if let Ok(word) = self.encode(&parameters) {
                    words.insert(word);
                }
            }
        }
        words.into_iter().collect()
    }

    /// Produces a valid word whose register operands share one encoded value.
    pub fn alias_word(&self, value: u32) -> Option<u32> {
        let mut parameters = self.canonical_parameters();
        let mut changed = false;
        for (parameter, field) in parameters.iter_mut().zip(&self.operands) {
            if field.class == PpuOperandClass::Register {
                *parameter = value & field.maximum();
                changed = true;
            }
        }
        changed.then(|| self.encode(&parameters).ok()).flatten()
    }

    /// Produces exact-kind words by toggling non-operand bits.
    pub fn reserved_bit_words(&self) -> Vec<u32> {
        let original = crate::decode::decode(self.canonical_word).ok();
        let semantic_reserved = semantic_reserved_bits(self.kind);
        let operand_mask = self
            .operands
            .iter()
            .fold(0, |mask, field| mask | field.mask);
        let discriminator_mask = discriminator_bits(self.kind, self.form) & !operand_mask;
        (0..u32::BITS)
            .filter_map(|bit| {
                let bit_mask = 1u32 << bit;
                if operand_mask & bit_mask != 0 || discriminator_mask & bit_mask != 0 {
                    return None;
                }
                let candidate = self.canonical_word ^ bit_mask;
                let ignored_by_decoder = crate::decode::decode(candidate).ok() == original;
                let reserved_operand =
                    semantic_reserved & bit_mask != 0 && exact_kind(candidate) == Some(self.kind);
                (ignored_by_decoder || reserved_operand).then_some(candidate)
            })
            .collect()
    }

    /// Produces valid exact-kind words by clearing one encoded operand bit.
    pub fn shrink(&self, raw: u32) -> Vec<u32> {
        self.operands
            .iter()
            .flat_map(|field| (0..u32::BITS).map(move |bit| (field, bit)))
            .filter_map(|(field, bit)| {
                let bit_mask = 1u32 << bit;
                if field.mask & bit_mask == 0 || raw & bit_mask == 0 {
                    return None;
                }
                let candidate = raw & !bit_mask;
                let decoded = crate::decode::decode(candidate).ok()?;
                (decoded.fuzz_descriptor(candidate).kind == self.kind
                    && generation_operands_are_valid(decoded))
                .then_some(candidate)
            })
            .collect()
    }
}

/// Lists every standalone PPU instruction recipe in exact-kind order.
pub fn generation_descriptors() -> Vec<PpuGenerationDescriptor> {
    build_generation_descriptors()
}

/// Lists every standalone decoded kind without consulting generator recipes.
pub fn expected_generation_kinds() -> BTreeSet<PpuFuzzKind> {
    let mut expected = BTreeSet::new();
    for kind in PpuInstructionKind::VARIANTS {
        if !is_synthetic_kind(*kind)
            && !matches!(
                kind,
                PpuInstructionKind::Vx
                    | PpuInstructionKind::Va
                    | PpuInstructionKind::Fp59
                    | PpuInstructionKind::Fp63
            )
        {
            expected.insert(PpuFuzzKind::Ordinary(*kind));
        }
    }
    expected.extend(
        VxOp::iter()
            .filter(|op| *op != VxOp::Vxor)
            .map(PpuFuzzKind::Vx),
    );
    expected.extend(
        VaOp::iter()
            .filter(|op| *op != VaOp::Vsldoi)
            .map(PpuFuzzKind::Va),
    );
    expected.extend(Fp59Op::iter().map(PpuFuzzKind::Fp59));
    expected.extend(Fp63Op::iter().map(PpuFuzzKind::Fp63));
    expected
}

fn is_synthetic_kind(kind: PpuInstructionKind) -> bool {
    use PpuInstructionKind as K;
    matches!(
        kind,
        K::Li
            | K::Mr
            | K::Slwi
            | K::Srwi
            | K::Clrlwi
            | K::Nop
            | K::CmpwZero
            | K::Clrldi
            | K::Sldi
            | K::Srdi
            | K::LwzCmpwi
            | K::LiStw
            | K::MflrStw
            | K::LwzMtlr
            | K::MflrStd
            | K::LdMtlr
            | K::StdStd
            | K::CmpwiBc
            | K::CmpwBc
            | K::Consumed
    )
}

/// Finds the generation recipe for a decoded word.
pub fn generation_descriptor(raw: u32) -> Option<PpuGenerationDescriptor> {
    let instruction = crate::decode::decode(raw).ok()?;
    let contract = instruction.fuzz_descriptor(raw);
    (contract.form != PpuEncodingForm::Synthetic).then(|| PpuGenerationDescriptor {
        kind: contract.kind,
        form: contract.form,
        sequence_class: sequence_class(contract.kind),
        sequence_flow: sequence_flow(contract.outcomes),
        sequence_dependency: sequence_dependency(contract.kind),
        canonical_word: raw,
        operands: operand_fields(raw, instruction, contract),
    })
}

const NO_EFFECTS: &[EffectKind] = &[];
const READ_EFFECTS: &[EffectKind] = &[EffectKind::SharedReadIntent];
// A successful committed-memory load-reserve step emits both effect packets.
const RESERVATION_READ_EFFECTS: &[EffectKind] =
    &[EffectKind::SharedReadIntent, EffectKind::ReservationAcquire];
const WRITE_EFFECTS: &[EffectKind] = &[EffectKind::SharedWriteIntent];
const ATOMIC_STORE_EFFECTS: &[EffectKind] = &[EffectKind::ConditionalStore];
const CLOCK_EFFECTS: &[EffectKind] = &[EffectKind::ClockRead];
const CONTINUE: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue];
const FAULT: &[PpuOutcomeClass] = &[PpuOutcomeClass::Fault];
const CONTINUE_OR_FAULT: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::Fault];
const LOAD: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::MemoryFault];
const STORE: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::BufferFull];
const ATOMIC_LOAD: &[PpuOutcomeClass] = &[
    PpuOutcomeClass::Continue,
    PpuOutcomeClass::Fault,
    PpuOutcomeClass::MemoryFault,
];
const ATOMIC_STORE: &[PpuOutcomeClass] = &[
    PpuOutcomeClass::Continue,
    PpuOutcomeClass::Fault,
    PpuOutcomeClass::MemoryFault,
    PpuOutcomeClass::BufferFull,
];
const UNCONDITIONAL_BRANCH: &[PpuOutcomeClass] = &[PpuOutcomeClass::Branch];
const BRANCH: &[PpuOutcomeClass] = &[PpuOutcomeClass::Continue, PpuOutcomeClass::Branch];
const SYSCALL: &[PpuOutcomeClass] = &[PpuOutcomeClass::Syscall];
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
        let (effects, outcomes) = effect_and_outcome(instruction_kind);
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
    // [Le2014 p:147 s:Abstract] A metamorphic partner is equivalent only under stated input conditions.
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

fn build_generation_descriptors() -> Vec<PpuGenerationDescriptor> {
    let mut words = BTreeMap::new();

    // Decoder discriminators use:
    // - the primary opcode;
    // - the low 11 bits;
    // - the two middle five-bit fields in XFX forms.
    // The finite scan does not sample operand data.
    let middle_templates = [
        0,
        1 << 11,
        31 << 11,
        1 << 16,
        31 << 16,
        1 << 21,
        31 << 21,
        (3 << 11) | (4 << 16) | (5 << 21),
    ];
    for primary in 0..64u32 {
        for suffix in 0..2048u32 {
            for middle in middle_templates {
                retain_generation_word((primary << 26) | middle | suffix, &mut words);
            }
        }
    }
    for selector in 0..1024u32 {
        for suffix in 0..2048u32 {
            retain_generation_word((31 << 26) | (selector << 11) | suffix, &mut words);
        }
    }

    // The family enums define the exact operations independently of the raw-word scan.
    for op in VxOp::iter() {
        retain_generation_word((4 << 26) | u32::from(op as u16), &mut words);
    }
    for op in VaOp::iter() {
        retain_generation_word((4 << 26) | u32::from(op as u8), &mut words);
    }
    for op in Fp59Op::iter() {
        retain_generation_word((59 << 26) | (u32::from(op as u16) << 1), &mut words);
    }
    for op in Fp63Op::iter() {
        retain_generation_word((63 << 26) | (u32::from(op as u16) << 1), &mut words);
    }

    words
        .into_iter()
        .filter_map(|(kind, canonical_word)| {
            let instruction = crate::decode::decode(canonical_word).ok()?;
            let contract = instruction.fuzz_descriptor(canonical_word);
            Some(PpuGenerationDescriptor {
                kind,
                form: contract.form,
                sequence_class: sequence_class(kind),
                sequence_flow: sequence_flow(contract.outcomes),
                sequence_dependency: sequence_dependency(kind),
                canonical_word,
                operands: operand_fields(canonical_word, instruction, contract),
            })
        })
        .collect()
}

fn retain_generation_word(raw: u32, words: &mut BTreeMap<PpuFuzzKind, u32>) {
    let Ok(instruction) = crate::decode::decode(raw) else {
        return;
    };
    let descriptor = instruction.fuzz_descriptor(raw);
    if descriptor.form != PpuEncodingForm::Synthetic && generation_operands_are_valid(instruction) {
        let canonical = words.entry(descriptor.kind).or_insert(raw);
        // [PPC-Book1 p:66 s:3.3.11] ori is primary-24 D-form. Decoder
        // aliases such as isync and cache hints must not hide its operands.
        if descriptor.kind == PpuFuzzKind::Ordinary(PpuInstructionKind::Ori) && raw >> 26 == 24 {
            *canonical = raw;
        }
    }
}

fn operand_fields(
    raw: u32,
    instruction: PpuInstruction,
    descriptor: PpuFuzzDescriptor,
) -> Vec<PpuOperandField> {
    field_candidates(descriptor)
        .into_iter()
        .filter_map(|(mask, class)| {
            let active = (active_bits(raw, instruction, descriptor.kind)
                | decoder_opaque_operand_bits(descriptor.kind))
                & mask;
            (active != 0).then_some(PpuOperandField {
                class,
                mask: active,
            })
        })
        .collect()
}

fn active_bits(raw: u32, instruction: PpuInstruction, kind: PpuFuzzKind) -> u32 {
    (0..u32::BITS).fold(0, |mask, bit| {
        let candidate = raw ^ (1u32 << bit);
        match crate::decode::decode(candidate) {
            Ok(decoded)
                if decoded != instruction && decoded.fuzz_descriptor(candidate).kind == kind =>
            {
                mask | (1u32 << bit)
            }
            _ => mask,
        }
    })
}

fn field_candidates(descriptor: PpuFuzzDescriptor) -> Vec<(u32, PpuOperandClass)> {
    use PpuEncodingForm as F;
    use PpuFuzzKind as K;
    use PpuInstructionKind as I;
    use PpuOperandClass as C;

    const BIT_0: u32 = 0x0000_0001;
    const BITS_1_5: u32 = 0x0000_003e;
    const BITS_5_10: u32 = 0x0000_07e0;
    const BITS_6_10: u32 = 0x0000_07c0;
    const BITS_11_15: u32 = 0x0000_f800;
    const BITS_16_20: u32 = 0x001f_0000;
    const BITS_21_25: u32 = 0x03e0_0000;

    match descriptor.form {
        // [PPC-Book1 p:8 s:1.7.4] Compare-immediate uses BF in the
        // D-form RT slot; the other D-form instructions use a register.
        F::D => match descriptor.kind {
            K::Ordinary(I::Cmpwi | I::Cmplwi | I::Cmpdi | I::Cmpldi) => vec![
                (0x0000_ffff, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Condition),
            ],
            _ => vec![
                (0x0000_ffff, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
        },
        F::Ds => vec![
            (0x0000_fffc, C::Immediate),
            (BITS_16_20, C::Register),
            (BITS_21_25, C::Register),
        ],
        F::I => vec![
            (0x0000_0001, C::Flag),
            (0x0000_0002, C::Flag),
            (0x03ff_fffc, C::Immediate),
        ],
        F::B => vec![
            (0x0000_0001, C::Flag),
            (0x0000_0002, C::Flag),
            (0x0000_fffc, C::Immediate),
            (0x001f_0000, C::Condition),
            (0x03e0_0000, C::Condition),
        ],
        // [PPC-Book1 p:9 s:1.7.6] X-form reuses its register slots for
        // condition, shift, count, and selector operands in named forms.
        F::X => match descriptor.kind {
            K::Ordinary(I::Cmpw | I::Cmplw | I::Cmpd | I::Cmpld | I::Mcrxr) => vec![
                (BITS_11_15, C::Register),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Condition),
            ],
            K::Ordinary(I::Tw | I::Td) => vec![
                (BITS_11_15, C::Register),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Selector),
            ],
            K::Ordinary(I::Lswi | I::Stswi) => vec![
                (BITS_11_15, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
            K::Ordinary(I::Srawi) => vec![
                (BIT_0, C::Flag),
                (BITS_11_15, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
            K::Ordinary(I::Sradi) => vec![
                (BIT_0, C::Flag),
                (0x0000_f802, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
            K::Ordinary(I::Mfocrf | I::Mtcrf | I::Mtocrf) => {
                vec![(0x000f_f000, C::Selector), (BITS_21_25, C::Register)]
            }
            _ => vec![
                (BIT_0, C::Flag),
                (0x0000_0400, C::Flag),
                (BITS_11_15, C::Register),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
        },
        // [PPC-Book1 p:9 s:1.7.7] XL-form operands are CR selectors or
        // branch conditions; it has no general-register operand slot.
        F::Xl => match descriptor.kind {
            K::Ordinary(I::Bclr | I::Bcctr) => vec![
                (BIT_0, C::Flag),
                (0x0000_1800, C::Selector),
                (BITS_16_20, C::Condition),
                (BITS_21_25, C::Condition),
            ],
            _ => vec![
                (BIT_0, C::Flag),
                (BITS_6_10, C::Condition),
                (BITS_11_15, C::Condition),
                (BITS_16_20, C::Condition),
                (BITS_21_25, C::Condition),
            ],
        },
        // [PPC-Book1 p:10 s:1.7.13] The third M-form slot is SH for
        // immediate rotates and RB for rlwnm.
        F::M => vec![
            (BIT_0, C::Flag),
            (BITS_1_5, C::Immediate),
            (BITS_6_10, C::Immediate),
            (
                BITS_11_15,
                if descriptor.kind == K::Ordinary(I::Rlwnm) {
                    C::Register
                } else {
                    C::Immediate
                },
            ),
            (BITS_16_20, C::Register),
            (BITS_21_25, C::Register),
        ],
        // [PPC-Book1 p:10 s:1.7.14] MD-form splits SH and its mask
        // bound; MDS-form replaces SH with RB.
        F::Md => match descriptor.kind {
            K::Ordinary(I::Rldcl | I::Rldcr) => vec![
                (BIT_0, C::Flag),
                (BITS_5_10, C::Immediate),
                (BITS_11_15, C::Register),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
            _ => vec![
                (BIT_0, C::Flag),
                (0x0000_f802, C::Immediate),
                (BITS_5_10, C::Immediate),
                (BITS_16_20, C::Register),
                (BITS_21_25, C::Register),
            ],
        },
        F::Vector => vector_field_candidates(descriptor.kind),
        F::Float => float_field_candidates(descriptor.kind),
        F::SystemCall => vec![(0x0000_0fe0, C::Selector)],
        F::Synthetic => Vec::new(),
    }
}

fn vector_field_candidates(kind: PpuFuzzKind) -> Vec<(u32, PpuOperandClass)> {
    use PpuOperandClass as C;

    const VC: u32 = 0x0000_07c0;
    const VB: u32 = 0x0000_f800;
    const VA: u32 = 0x001f_0000;
    const VD: u32 = 0x03e0_0000;

    // [AltiVec-PEM p:A-21 s:A.5] VA/VX forms reserve unused slots and
    // place splat immediates in vA; VXR compares carry Rc in raw bit 10.
    match kind {
        PpuFuzzKind::Ordinary(PpuInstructionKind::Vsldoi) => vec![
            (0x0000_03c0, C::Immediate),
            (VB, C::Register),
            (VA, C::Register),
            (VD, C::Register),
        ],
        PpuFuzzKind::Ordinary(PpuInstructionKind::Vxor) => {
            vec![(VB, C::Register), (VA, C::Register), (VD, C::Register)]
        }
        PpuFuzzKind::Vx(op) => {
            let mut fields = Vec::new();
            if op.is_vxr_compare() {
                fields.push((0x0000_0400, C::Flag));
            }
            match op.shape() {
                VxShape::VdVaVb => {
                    fields.extend([(VB, C::Register), (VA, C::Register), (VD, C::Register)]);
                }
                VxShape::VdVb => fields.extend([(VB, C::Register), (VD, C::Register)]),
                VxShape::VdVbUimm => {
                    fields.extend([(VB, C::Register), (VA, C::Immediate), (VD, C::Register)]);
                }
                VxShape::VdSimm => {
                    fields.extend([(VA, C::Immediate), (VD, C::Register)]);
                }
            }
            fields
        }
        PpuFuzzKind::Va(op) => match op.shape() {
            VaShape::VdVaVbVc | VaShape::VdVaVcVb => vec![
                (VC, C::Register),
                (VB, C::Register),
                (VA, C::Register),
                (VD, C::Register),
            ],
            VaShape::VdVaVbShb => vec![
                (0x0000_03c0, C::Immediate),
                (VB, C::Register),
                (VA, C::Register),
                (VD, C::Register),
            ],
        },
        _ => Vec::new(),
    }
}

fn float_field_candidates(kind: PpuFuzzKind) -> Vec<(u32, PpuOperandClass)> {
    use PpuOperandClass as C;

    const RC: (u32, C) = (0x0000_0001, C::Flag);
    const FRC: (u32, C) = (0x0000_07c0, C::Register);
    const FRB: (u32, C) = (0x0000_f800, C::Register);
    const FRA: (u32, C) = (0x001f_0000, C::Register);
    const FRT: (u32, C) = (0x03e0_0000, C::Register);

    // [PPC-Book1 p:9 s:1.7.6] The floating X forms reuse register
    // slots for CR/FPSCR selectors and reserve every unused slot.
    match kind {
        PpuFuzzKind::Fp59(op) => match op.shape() {
            Fp59Shape::FrtFraFrb => vec![RC, FRB, FRA, FRT],
            Fp59Shape::FrtFrb => vec![RC, FRB, FRT],
            Fp59Shape::FrtFraFrc => vec![RC, FRC, FRA, FRT],
            Fp59Shape::FrtFraFrcFrb => vec![RC, FRC, FRB, FRA, FRT],
        },
        PpuFuzzKind::Fp63(op) => match op.shape() {
            Fp63Shape::FrtFraFrb => vec![RC, FRB, FRA, FRT],
            Fp63Shape::FrtFrb => vec![RC, FRB, FRT],
            Fp63Shape::FrtFraFrc => vec![RC, FRC, FRA, FRT],
            Fp63Shape::FrtFraFrcFrb => vec![RC, FRC, FRB, FRA, FRT],
            Fp63Shape::CrfFraFrb => vec![FRB, FRA, (0x0380_0000, C::Condition)],
            Fp63Shape::Frt => vec![RC, FRT],
            Fp63Shape::CrfCrf => vec![(0x001c_0000, C::Condition), (0x0380_0000, C::Condition)],
            Fp63Shape::CrfImm => vec![RC, (0x0000_f000, C::Immediate), (0x0380_0000, C::Condition)],
            Fp63Shape::Crb => vec![RC, (0x03e0_0000, C::Selector)],
            Fp63Shape::FmFrb => vec![RC, FRB, (0x01fe_0000, C::Selector)],
        },
        _ => Vec::new(),
    }
}

fn semantic_reserved_bits(kind: PpuFuzzKind) -> u32 {
    // Some family decoders retain every physical slot so execution can share
    // one variant. The form shape remains authoritative for unused slots.
    match kind {
        PpuFuzzKind::Vx(op) => match op.shape() {
            VxShape::VdVb => 0x001f_0000,
            VxShape::VdSimm => 0x0000_f800,
            VxShape::VdVaVb | VxShape::VdVbUimm => 0,
        },
        // [PPC-Book1 p:10 s:1.7.12] A-form diagrams mark the unused
        // FRA, FRB, or FRC slots as reserved for the corresponding shape.
        PpuFuzzKind::Fp59(op) => match op.shape() {
            Fp59Shape::FrtFraFrb => 0x0000_07c0,
            Fp59Shape::FrtFrb => 0x001f_07c0,
            Fp59Shape::FrtFraFrc => 0x0000_f800,
            Fp59Shape::FrtFraFrcFrb => 0,
        },
        // [PPC-Book1 p:9 s:1.7.6] Floating X/XFL forms reserve the
        // unused portions of their CR, FPSCR, and register slots.
        PpuFuzzKind::Fp63(op) => match op.shape() {
            Fp63Shape::FrtFraFrb => 0x0000_07c0,
            Fp63Shape::FrtFrb if matches!(op, Fp63Op::Fsqrt | Fp63Op::Frsqrte) => 0x001f_07c0,
            Fp63Shape::FrtFrb => 0x001f_0000,
            Fp63Shape::FrtFraFrc => 0x0000_f800,
            Fp63Shape::FrtFraFrcFrb => 0,
            Fp63Shape::CrfFraFrb => 0x0060_0001,
            Fp63Shape::Frt => 0x001f_f800,
            Fp63Shape::CrfCrf => 0x0063_f801,
            Fp63Shape::CrfImm => 0x007f_0800,
            Fp63Shape::Crb => 0x001f_f800,
            Fp63Shape::FmFrb => 0x0201_0000,
        },
        _ => 0,
    }
}

fn decoder_opaque_operand_bits(kind: PpuFuzzKind) -> u32 {
    // [PPC-Book1 p:10 s:1.7.16] BH is an architected branch-hint
    // selector even though the deterministic decoder does not retain it.
    match kind {
        PpuFuzzKind::Ordinary(PpuInstructionKind::Bclr | PpuInstructionKind::Bcctr) => 0x0000_1800,
        _ => 0,
    }
}

fn discriminator_bits(kind: PpuFuzzKind, form: PpuEncodingForm) -> u32 {
    0xfc00_0000
        | match form {
            PpuEncodingForm::Ds => 0x0000_0003,
            PpuEncodingForm::X | PpuEncodingForm::Xl => 0x0000_07fe,
            PpuEncodingForm::Md => match kind {
                PpuFuzzKind::Ordinary(PpuInstructionKind::Rldcl | PpuInstructionKind::Rldcr) => {
                    0x0000_001e
                }
                _ => 0x0000_001c,
            },
            PpuEncodingForm::Vector => match kind {
                PpuFuzzKind::Ordinary(PpuInstructionKind::Vsldoi) | PpuFuzzKind::Va(_) => {
                    0x0000_003f
                }
                _ => 0x0000_07ff,
            },
            PpuEncodingForm::Float => match kind {
                PpuFuzzKind::Fp59(_) => 0x0000_003e,
                PpuFuzzKind::Fp63(
                    Fp63Op::Fdiv
                    | Fp63Op::Fsub
                    | Fp63Op::Fadd
                    | Fp63Op::Fsqrt
                    | Fp63Op::Fsel
                    | Fp63Op::Fmul
                    | Fp63Op::Frsqrte
                    | Fp63Op::Fmsub
                    | Fp63Op::Fmadd
                    | Fp63Op::Fnmsub
                    | Fp63Op::Fnmadd,
                ) => 0x0000_003e,
                _ => 0x0000_07fe,
            },
            PpuEncodingForm::SystemCall => 0x0000_0002,
            PpuEncodingForm::D
            | PpuEncodingForm::I
            | PpuEncodingForm::B
            | PpuEncodingForm::M
            | PpuEncodingForm::Synthetic => 0,
        }
}

fn generation_operands_are_valid(instruction: PpuInstruction) -> bool {
    use PpuInstruction as I;

    match instruction {
        // [PPC-Book1 p:33 s:3.3.2] A fixed-point load-update form
        // requires a nonzero RA distinct from RT.
        I::Lhau { rt, ra, .. }
        | I::Lwzu { rt, ra, .. }
        | I::Lbzu { rt, ra, .. }
        | I::Lhzu { rt, ra, .. }
        | I::Ldu { rt, ra, .. }
        | I::Lwzux { rt, ra, .. }
        | I::Lbzux { rt, ra, .. }
        | I::Lhzux { rt, ra, .. }
        | I::Ldux { rt, ra, .. }
        | I::Lhaux { rt, ra, .. }
        | I::Lwaux { rt, ra, .. } => ra != 0 && ra != rt,
        // [PPC-Book1 p:46 s:3.3.5] lmw is invalid when RA is zero or
        // names any register in the RT..=31 load range.
        I::Lmw { rt, ra, .. } => ra != 0 && ra < rt,
        // [PPC-Book1 p:48 s:3.3.6] lswi is invalid when RA is zero or
        // names one of the wrapping destination registers.
        I::Lswi { rt, ra, nb } => {
            let bytes = if nb == 0 { 32 } else { nb };
            let registers = bytes.div_ceil(4);
            ra != 0 && !(0..registers).any(|index| rt.wrapping_add(index) & 31 == ra)
        }
        // [PPC-Book1 p:48 s:3.3.6] lswx is always invalid when the
        // first destination names either address register.
        I::Lswx { rt, ra, rb } => rt != ra && rt != rb,
        // [PPC-Book1 p:104 s:4.6.2] Single-precision load-update requires nonzero RA.
        // [PPC-Book1 p:105 s:4.6.2] Double-precision load-update requires nonzero RA.
        // FRT is in a separate register file, so it may share RA's number.
        I::Lfsu { ra, .. } | I::Lfdu { ra, .. } | I::Lfsux { ra, .. } | I::Lfdux { ra, .. } => {
            ra != 0
        }
        // [PPC-Book1 p:40 s:3.3.3] Fixed-point store-update permits
        // RS=RA but requires nonzero RA.
        I::Stwu { ra, .. }
        | I::Stdu { ra, .. }
        | I::Stbu { ra, .. }
        | I::Sthu { ra, .. }
        | I::Stdux { ra, .. }
        | I::Sthux { ra, .. }
        | I::Stwux { ra, .. }
        | I::Stbux { ra, .. } => ra != 0,
        // [PPC-Book1 p:107 s:4.6] Single-precision store-update requires nonzero RA.
        // [PPC-Book1 p:108 s:4.6] Double-precision store-update requires nonzero RA.
        I::Stfsu { ra, .. } | I::Stfdu { ra, .. } | I::Stfsux { ra, .. } | I::Stfdux { ra, .. } => {
            ra != 0
        }
        // [PPC-Book1 p:25 s:2.4] bcctr cannot request CTR decrement.
        I::Bcctr { bo, .. } => bo & 0x04 != 0,
        _ => true,
    }
}

fn exact_kind(raw: u32) -> Option<PpuFuzzKind> {
    let instruction = crate::decode::decode(raw).ok()?;
    Some(instruction.fuzz_descriptor(raw).kind)
}

fn sequence_class(kind: PpuFuzzKind) -> PpuSequenceClass {
    match kind {
        PpuFuzzKind::Ordinary(PpuInstructionKind::Lswx) => PpuSequenceClass::ReadsXerByteCount,
        PpuFuzzKind::Ordinary(PpuInstructionKind::Mtxer) => PpuSequenceClass::ReplacesXer,
        _ => PpuSequenceClass::Independent,
    }
}

fn sequence_flow(outcomes: &[PpuOutcomeClass]) -> PpuSequenceFlow {
    if outcomes.contains(&PpuOutcomeClass::Branch) {
        PpuSequenceFlow::ControlTransfer
    } else if outcomes.contains(&PpuOutcomeClass::Syscall) || outcomes == FAULT {
        PpuSequenceFlow::Terminal
    } else if outcomes != CONTINUE {
        PpuSequenceFlow::StateDependent
    } else {
        PpuSequenceFlow::Linear
    }
}

fn sequence_dependency(kind: PpuFuzzKind) -> Option<PpuSequenceDependency> {
    // [PPC-Book1 p:66 s:3.3.13] These logical-immediate forms read RS and replace RA.
    matches!(
        kind,
        PpuFuzzKind::Ordinary(
            PpuInstructionKind::Ori
                | PpuInstructionKind::Oris
                | PpuInstructionKind::Xori
                | PpuInstructionKind::Xoris
        )
    )
    .then_some(PpuSequenceDependency::GeneralPurposeRegister)
}

fn extract_bits(word: u32, mask: u32) -> u32 {
    let mut packed = 0;
    let mut destination = 0;
    for source in 0..u32::BITS {
        let bit = 1u32 << source;
        if mask & bit != 0 {
            if word & bit != 0 {
                packed |= 1u32 << destination;
            }
            destination += 1;
        }
    }
    packed
}

fn deposit_bits(value: u32, mask: u32) -> u32 {
    let mut deposited = 0;
    let mut source = 0;
    for destination in 0..u32::BITS {
        let bit = 1u32 << destination;
        if mask & bit != 0 {
            if value & (1u32 << source) != 0 {
                deposited |= bit;
            }
            source += 1;
        }
    }
    deposited
}

fn low_mask(bits: u32) -> u32 {
    1u32.checked_shl(bits).map_or(u32::MAX, |limit| limit - 1)
}

/// Clear one raw bit and retain only a decodable encoding of the exact same kind.
pub fn simplify_bit(raw: u32, bit: u8) -> Option<u32> {
    if bit >= u32::BITS as u8 || raw & (1u32 << bit) == 0 {
        return None;
    }
    let instruction = crate::decode::decode(raw).ok()?;
    let kind = instruction.fuzz_descriptor(raw).kind;
    let candidate = raw & !(1u32 << bit);
    let decoded = crate::decode::decode(candidate).ok()?;
    (decoded.fuzz_descriptor(candidate).kind == kind).then_some(candidate)
}

/// Produces valid exact-kind encodings by clearing one set operand bit.
pub fn simplify_encoding(raw: u32) -> Vec<u32> {
    generation_descriptor(raw).map_or_else(Vec::new, |descriptor| descriptor.shrink(raw))
}

fn fuzz_kind(instruction: PpuInstruction) -> PpuFuzzKind {
    match instruction {
        PpuInstruction::Vx { op, .. } => PpuFuzzKind::Vx(op),
        PpuInstruction::Va { op, .. } => PpuFuzzKind::Va(op),
        PpuInstruction::Fp59 { op, .. } => PpuFuzzKind::Fp59(op),
        PpuInstruction::Fp63 { op, .. } => PpuFuzzKind::Fp63(op),
        ordinary => PpuFuzzKind::Ordinary(PpuInstructionKind::from(ordinary)),
    }
}

fn form_for_word(kind: PpuInstructionKind, raw: u32) -> PpuEncodingForm {
    use PpuInstructionKind as K;
    if matches!(
        kind,
        K::Li
            | K::Mr
            | K::Slwi
            | K::Srwi
            | K::Clrlwi
            | K::Nop
            | K::CmpwZero
            | K::Clrldi
            | K::Sldi
            | K::Srdi
            | K::LwzCmpwi
            | K::LiStw
            | K::MflrStw
            | K::LwzMtlr
            | K::MflrStd
            | K::LdMtlr
            | K::StdStd
            | K::CmpwiBc
            | K::CmpwBc
            | K::Consumed
    ) {
        return PpuEncodingForm::Synthetic;
    }
    match raw >> 26 {
        4 => PpuEncodingForm::Vector,
        16 => PpuEncodingForm::B,
        18 => PpuEncodingForm::I,
        19 => PpuEncodingForm::Xl,
        20 | 21 | 23 => PpuEncodingForm::M,
        30 => PpuEncodingForm::Md,
        31 => PpuEncodingForm::X,
        58 | 62 => PpuEncodingForm::Ds,
        59 | 63 => PpuEncodingForm::Float,
        17 => PpuEncodingForm::SystemCall,
        _ => PpuEncodingForm::D,
    }
}

fn effect_and_outcome(
    kind: PpuInstructionKind,
) -> (&'static [EffectKind], &'static [PpuOutcomeClass]) {
    use PpuInstructionKind as K;
    match kind {
        K::Lwz
        | K::Lbz
        | K::Lhz
        | K::Lha
        | K::Lhau
        | K::Lmw
        | K::Lwzu
        | K::Lbzu
        | K::Lhzu
        | K::Ldu
        | K::Ld
        | K::Lwa
        | K::Lwzx
        | K::Lbzx
        | K::Ldx
        | K::Lhzx
        | K::Lwzux
        | K::Lbzux
        | K::Lhzux
        | K::Ldux
        | K::Lhax
        | K::Lhaux
        | K::Lwax
        | K::Lwaux
        | K::Lswi
        | K::Lswx
        | K::Ldbrx
        | K::Lwbrx
        | K::Lhbrx
        | K::Lvlx
        | K::Lvrx
        | K::Lvlxl
        | K::Lvrxl
        | K::Lvsl
        | K::Lvebx
        | K::Lvsr
        | K::Lvehx
        | K::Lvewx
        | K::Lvx
        | K::Lvxl
        | K::Lfs
        | K::Lfsu
        | K::Lfd
        | K::Lfdu
        | K::Lfsx
        | K::Lfsux
        | K::Lfdx
        | K::Lfdux
        | K::LwzCmpwi
        | K::LwzMtlr
        | K::LdMtlr => (READ_EFFECTS, LOAD),
        K::Stw
        | K::Stwu
        | K::Stdu
        | K::Stb
        | K::Stbu
        | K::Stmw
        | K::Sth
        | K::Sthu
        | K::Std
        | K::Stwx
        | K::Stdx
        | K::Stdux
        | K::Stbx
        | K::Sthx
        | K::Sthux
        | K::Stwux
        | K::Stbux
        | K::Stswi
        | K::Stswx
        | K::Sdbrx
        | K::Stwbrx
        | K::Sthbrx
        | K::Stvlx
        | K::Stvrx
        | K::Stvlxl
        | K::Stvrxl
        | K::Stvebx
        | K::Stvehx
        | K::Stvewx
        | K::Stvx
        | K::Stvxl
        | K::Stfs
        | K::Stfd
        | K::Stfsu
        | K::Stfdu
        | K::Stfiwx
        | K::Stfsx
        | K::Stfsux
        | K::Stfdx
        | K::Stfdux
        | K::Dcbz
        | K::LiStw
        | K::MflrStw
        | K::MflrStd
        | K::StdStd => (WRITE_EFFECTS, STORE),
        K::Ldarx | K::Lwarx => (RESERVATION_READ_EFFECTS, ATOMIC_LOAD),
        K::Stdcx | K::Stwcx => (ATOMIC_STORE_EFFECTS, ATOMIC_STORE),
        K::Mftb | K::Mftbu => (CLOCK_EFFECTS, CONTINUE),
        K::B => (NO_EFFECTS, UNCONDITIONAL_BRANCH),
        K::Bc | K::Bclr | K::Bcctr | K::CmpwiBc | K::CmpwBc => (NO_EFFECTS, BRANCH),
        K::Sc => (NO_EFFECTS, SYSCALL),
        K::Popcntb => (NO_EFFECTS, FAULT),
        K::Tw | K::Td | K::Mfocrf | K::Mtocrf | K::Vx | K::Va => (NO_EFFECTS, CONTINUE_OR_FAULT),
        _ => (NO_EFFECTS, CONTINUE),
    }
}

fn classify_kind(kind: PpuInstructionKind) {
    match kind {
        PpuInstructionKind::B
        | PpuInstructionKind::Lwz
        | PpuInstructionKind::Lbz
        | PpuInstructionKind::Lhz
        | PpuInstructionKind::Lha
        | PpuInstructionKind::Lhau
        | PpuInstructionKind::Lmw
        | PpuInstructionKind::Lwzu
        | PpuInstructionKind::Lbzu
        | PpuInstructionKind::Lhzu
        | PpuInstructionKind::Ldu
        | PpuInstructionKind::Ld
        | PpuInstructionKind::Lwa
        | PpuInstructionKind::Stw
        | PpuInstructionKind::Stwu
        | PpuInstructionKind::Stdu
        | PpuInstructionKind::Stb
        | PpuInstructionKind::Stbu
        | PpuInstructionKind::Stmw
        | PpuInstructionKind::Sth
        | PpuInstructionKind::Sthu
        | PpuInstructionKind::Std
        | PpuInstructionKind::Addi
        | PpuInstructionKind::Addis
        | PpuInstructionKind::Subfic
        | PpuInstructionKind::Mulli
        | PpuInstructionKind::Addic
        | PpuInstructionKind::AddicDot
        | PpuInstructionKind::Add
        | PpuInstructionKind::Or
        | PpuInstructionKind::Subf
        | PpuInstructionKind::Subfc
        | PpuInstructionKind::Subfe
        | PpuInstructionKind::Neg
        | PpuInstructionKind::Mullw
        | PpuInstructionKind::Mulhwu
        | PpuInstructionKind::Mulhw
        | PpuInstructionKind::Mulhdu
        | PpuInstructionKind::Mulhd
        | PpuInstructionKind::Adde
        | PpuInstructionKind::Addze
        | PpuInstructionKind::Subfze
        | PpuInstructionKind::Subfme
        | PpuInstructionKind::Addme
        | PpuInstructionKind::Mulld
        | PpuInstructionKind::Ldarx
        | PpuInstructionKind::Stdcx
        | PpuInstructionKind::Lwarx
        | PpuInstructionKind::Stwcx
        | PpuInstructionKind::Xori
        | PpuInstructionKind::Xoris
        | PpuInstructionKind::Divw
        | PpuInstructionKind::Divwu
        | PpuInstructionKind::Divd
        | PpuInstructionKind::Divdu
        | PpuInstructionKind::And
        | PpuInstructionKind::Andc
        | PpuInstructionKind::Nor
        | PpuInstructionKind::Xor
        | PpuInstructionKind::Eqv
        | PpuInstructionKind::Nand
        | PpuInstructionKind::AndiDot
        | PpuInstructionKind::AndisDot
        | PpuInstructionKind::Slw
        | PpuInstructionKind::Srw
        | PpuInstructionKind::Srawi
        | PpuInstructionKind::Sraw
        | PpuInstructionKind::Srad
        | PpuInstructionKind::Sradi
        | PpuInstructionKind::Sld
        | PpuInstructionKind::Srd
        | PpuInstructionKind::Cntlzw
        | PpuInstructionKind::Cntlzd
        | PpuInstructionKind::Popcntb
        | PpuInstructionKind::Tw
        | PpuInstructionKind::Td
        | PpuInstructionKind::Mcrxr
        | PpuInstructionKind::Orc
        | PpuInstructionKind::Extsh
        | PpuInstructionKind::Extsb
        | PpuInstructionKind::Extsw
        | PpuInstructionKind::Ori
        | PpuInstructionKind::Oris
        | PpuInstructionKind::Cmpwi
        | PpuInstructionKind::Cmplwi
        | PpuInstructionKind::Cmpdi
        | PpuInstructionKind::Cmpldi
        | PpuInstructionKind::Cmpw
        | PpuInstructionKind::Cmplw
        | PpuInstructionKind::Cmpd
        | PpuInstructionKind::Cmpld
        | PpuInstructionKind::Bc
        | PpuInstructionKind::Bclr
        | PpuInstructionKind::Bcctr
        | PpuInstructionKind::Mcrf
        | PpuInstructionKind::Crand
        | PpuInstructionKind::Crandc
        | PpuInstructionKind::Cror
        | PpuInstructionKind::Crorc
        | PpuInstructionKind::Crxor
        | PpuInstructionKind::Crnand
        | PpuInstructionKind::Crnor
        | PpuInstructionKind::Creqv
        | PpuInstructionKind::Lwzx
        | PpuInstructionKind::Lbzx
        | PpuInstructionKind::Ldx
        | PpuInstructionKind::Lhzx
        | PpuInstructionKind::Stwx
        | PpuInstructionKind::Stdx
        | PpuInstructionKind::Stdux
        | PpuInstructionKind::Stbx
        | PpuInstructionKind::Lwzux
        | PpuInstructionKind::Lbzux
        | PpuInstructionKind::Lhzux
        | PpuInstructionKind::Ldux
        | PpuInstructionKind::Lhax
        | PpuInstructionKind::Lhaux
        | PpuInstructionKind::Lwax
        | PpuInstructionKind::Lwaux
        | PpuInstructionKind::Sthx
        | PpuInstructionKind::Sthux
        | PpuInstructionKind::Stwux
        | PpuInstructionKind::Stbux
        | PpuInstructionKind::Lswi
        | PpuInstructionKind::Lswx
        | PpuInstructionKind::Stswi
        | PpuInstructionKind::Stswx
        | PpuInstructionKind::Ldbrx
        | PpuInstructionKind::Lwbrx
        | PpuInstructionKind::Lhbrx
        | PpuInstructionKind::Sdbrx
        | PpuInstructionKind::Stwbrx
        | PpuInstructionKind::Sthbrx
        | PpuInstructionKind::Mftb
        | PpuInstructionKind::Mftbu
        | PpuInstructionKind::Mfcr
        | PpuInstructionKind::Mtcrf
        | PpuInstructionKind::Mfocrf
        | PpuInstructionKind::Mtocrf
        | PpuInstructionKind::Mflr
        | PpuInstructionKind::Mtlr
        | PpuInstructionKind::Mfctr
        | PpuInstructionKind::Mtctr
        | PpuInstructionKind::Mfxer
        | PpuInstructionKind::Mtxer
        | PpuInstructionKind::Mfvrsave
        | PpuInstructionKind::Mtvrsave
        | PpuInstructionKind::Rlwinm
        | PpuInstructionKind::Rlwimi
        | PpuInstructionKind::Rlwnm
        | PpuInstructionKind::Rldicl
        | PpuInstructionKind::Rldicr
        | PpuInstructionKind::Rldic
        | PpuInstructionKind::Rldimi
        | PpuInstructionKind::Rldcl
        | PpuInstructionKind::Rldcr
        | PpuInstructionKind::Vx
        | PpuInstructionKind::Va
        | PpuInstructionKind::Vxor
        | PpuInstructionKind::Vsldoi
        | PpuInstructionKind::Lvlx
        | PpuInstructionKind::Lvrx
        | PpuInstructionKind::Lvlxl
        | PpuInstructionKind::Lvrxl
        | PpuInstructionKind::Stvlx
        | PpuInstructionKind::Stvrx
        | PpuInstructionKind::Stvlxl
        | PpuInstructionKind::Stvrxl
        | PpuInstructionKind::Lvsl
        | PpuInstructionKind::Lvebx
        | PpuInstructionKind::Lvsr
        | PpuInstructionKind::Lvehx
        | PpuInstructionKind::Lvewx
        | PpuInstructionKind::Lvx
        | PpuInstructionKind::Stvebx
        | PpuInstructionKind::Stvehx
        | PpuInstructionKind::Stvewx
        | PpuInstructionKind::Lvxl
        | PpuInstructionKind::Stvx
        | PpuInstructionKind::Stvxl
        | PpuInstructionKind::Lfs
        | PpuInstructionKind::Lfsu
        | PpuInstructionKind::Lfd
        | PpuInstructionKind::Lfdu
        | PpuInstructionKind::Stfs
        | PpuInstructionKind::Stfd
        | PpuInstructionKind::Stfsu
        | PpuInstructionKind::Stfdu
        | PpuInstructionKind::Stfiwx
        | PpuInstructionKind::Lfsx
        | PpuInstructionKind::Lfsux
        | PpuInstructionKind::Lfdx
        | PpuInstructionKind::Lfdux
        | PpuInstructionKind::Stfsx
        | PpuInstructionKind::Stfsux
        | PpuInstructionKind::Stfdx
        | PpuInstructionKind::Stfdux
        | PpuInstructionKind::Fp63
        | PpuInstructionKind::Fp59
        | PpuInstructionKind::Li
        | PpuInstructionKind::Mr
        | PpuInstructionKind::Slwi
        | PpuInstructionKind::Srwi
        | PpuInstructionKind::Clrlwi
        | PpuInstructionKind::Nop
        | PpuInstructionKind::CmpwZero
        | PpuInstructionKind::Clrldi
        | PpuInstructionKind::Sldi
        | PpuInstructionKind::Srdi
        | PpuInstructionKind::LwzCmpwi
        | PpuInstructionKind::LiStw
        | PpuInstructionKind::MflrStw
        | PpuInstructionKind::LwzMtlr
        | PpuInstructionKind::MflrStd
        | PpuInstructionKind::LdMtlr
        | PpuInstructionKind::StdStd
        | PpuInstructionKind::CmpwiBc
        | PpuInstructionKind::CmpwBc
        | PpuInstructionKind::Consumed
        | PpuInstructionKind::Dcbz
        | PpuInstructionKind::Sc => {}
    }
}

#[cfg(test)]
#[path = "tests/fuzz_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/metamorphic_tests.rs"]
mod metamorphic_tests;
