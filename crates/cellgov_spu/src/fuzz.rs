//! Interpreter-owned contracts used by instruction fuzzers.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_effects::EffectKind;
use cellgov_ps3_abi::hw::spu;
#[cfg(test)]
use strum::VariantArray;

use crate::instruction::{SpuInstruction, SpuInstructionKind};

/// Encoding form used by an SPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuEncodingForm {
    /// Three-field register form.
    Rrr,
    /// Four-field register form.
    Rrrr,
    /// Register plus 7-bit immediate.
    Ri7,
    /// Register plus 10-bit immediate.
    Ri10,
    /// Register plus 16-bit immediate.
    Ri16,
    /// Register plus 18-bit immediate.
    Ri18,
    /// Branch encoding family.
    Branch,
    /// Channel encoding family.
    Channel,
    /// Control and hint encoding family.
    Control,
}

/// Observable SPU state used for comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuObservableState {
    /// Registers, local store, PC, channels, reservation, outcome, and effects.
    Complete,
}

/// Legal result class for one SPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuOutcomeClass {
    /// Ordinary completion.
    Continue,
    /// Explicit PC change.
    Branch,
    /// Runtime yield with effects.
    Yield,
    /// Caller-serviced committed-memory read.
    MemoryRead,
    /// Architectural fault.
    Fault,
}

impl SpuOutcomeClass {
    /// Classify an executor outcome exhaustively.
    pub fn from_outcome(outcome: &crate::exec::SpuStepOutcome) -> Self {
        match outcome {
            crate::exec::SpuStepOutcome::Continue => Self::Continue,
            crate::exec::SpuStepOutcome::Branch => Self::Branch,
            crate::exec::SpuStepOutcome::Yield { .. } => Self::Yield,
            crate::exec::SpuStepOutcome::MemoryRead { .. } => Self::MemoryRead,
            crate::exec::SpuStepOutcome::Fault(_) => Self::Fault,
        }
    }
}

/// Interpreter self-relation suitable for fuzz checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuMetamorphicRelation {
    /// Identical inputs give identical outputs.
    Deterministic,
}

/// Complete fuzz contract for one decoded SPU instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuFuzzDescriptor {
    /// Stable instruction identity.
    pub kind: SpuInstructionKind,
    /// Encoding form.
    pub form: SpuEncodingForm,
    /// State projection used for comparison.
    pub observable_state: SpuObservableState,
    /// Effect variants this instruction may return in a yield outcome.
    pub effects: &'static [EffectKind],
    /// Result classes accepted from execution.
    pub outcomes: &'static [SpuOutcomeClass],
    /// Relations that apply to this instruction.
    pub relations: &'static [SpuMetamorphicRelation],
    /// Whether the executor supports this decoded instruction.
    pub decoded_execution_supported: bool,
    /// Register input for a state-dependent instruction.
    pub state_input: Option<SpuStateInput>,
}

/// Input choices for one register that needs defined state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuStateInput {
    /// Register that receives the selected word.
    pub register: u8,
    /// Values that satisfy the execution precondition.
    pub values: &'static [u32],
    /// Value the generator selects more often for effect coverage.
    pub preferred: Option<u32>,
}

/// Semantic class of one encoded SPU operand field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpuOperandClass {
    /// Selects an SPU register.
    Register,
    /// Carries an immediate operand.
    Immediate,
    /// Selects a channel.
    Channel,
    /// Carries a one-bit encoding option.
    Flag,
}

/// Control-flow behavior relevant to bounded sequence generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuSequenceFlow {
    /// Execution normally advances to the next generated word.
    Linear,
    /// Execution depends on architectural state that earlier words can replace.
    StateDependent,
    /// Execution can select another program counter.
    ControlTransfer,
    /// Execution ends the generated sequence at this instruction.
    Terminal,
}

/// One packed operand field in an SPU encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuOperandField {
    /// Semantic operand class.
    pub class: SpuOperandClass,
    /// Bits occupied by the field in the instruction word.
    pub mask: u32,
}

impl SpuOperandField {
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

/// Interpreter-owned recipe for constructing one SPU instruction kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuGenerationDescriptor {
    /// Instruction kind generated by this recipe.
    pub kind: SpuInstructionKind,
    /// Encoding form.
    pub form: SpuEncodingForm,
    /// Control-flow behavior in a generated sequence.
    pub sequence_flow: SpuSequenceFlow,
    /// Channel selectors that the generator selects more often.
    pub channel_values: &'static [u32],
    /// Known decodable word used for canonical operand values.
    pub canonical_word: u32,
    /// Typed operand fields in low-to-high bit order.
    pub operands: Vec<SpuOperandField>,
}

/// Reports a structural SPU encoding request that conflicts with its descriptor.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpuGenerationError {
    /// The parameter stream did not contain one value per operand field.
    #[error("SPU generation expected {expected} operands but received {found}")]
    OperandCount {
        /// Operand count declared by the descriptor.
        expected: usize,
        /// Operand count supplied by the caller.
        found: usize,
    },
    /// The operands do not form a valid encoding of the selected kind.
    #[error("SPU operands do not encode a valid selected instruction kind")]
    InvalidOperands,
}

impl SpuGenerationDescriptor {
    /// Encodes one value per typed operand field.
    ///
    /// # Errors
    ///
    /// - If the number of values is incorrect, the method returns [`SpuGenerationError::OperandCount`].
    /// - If the operands are invalid, the method returns [`SpuGenerationError::InvalidOperands`].
    pub fn encode(&self, values: &[u32]) -> Result<u32, SpuGenerationError> {
        if values.len() != self.operands.len() {
            return Err(SpuGenerationError::OperandCount {
                expected: self.operands.len(),
                found: values.len(),
            });
        }
        if values
            .iter()
            .zip(&self.operands)
            .any(|(value, field)| *value > field.maximum())
        {
            return Err(SpuGenerationError::InvalidOperands);
        }
        let mut word = self.canonical_word;
        for (field, value) in self.operands.iter().zip(values) {
            word = (word & !field.mask) | deposit_bits(*value, field.mask);
        }
        if !operand_combination_is_valid(self.kind, word) {
            return Err(SpuGenerationError::InvalidOperands);
        }
        exact_kind(word)
            .filter(|kind| *kind == self.kind)
            .map(|_| word)
            .ok_or(SpuGenerationError::InvalidOperands)
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
            if field.class != SpuOperandClass::Immediate {
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
            if field.class == SpuOperandClass::Register {
                *parameter = value & field.maximum();
                changed = true;
            }
        }
        changed.then(|| self.encode(&parameters).ok()).flatten()
    }

    /// Produces same-kind words by toggling non-operand bits.
    pub fn reserved_bit_words(&self) -> Vec<u32> {
        let original = crate::decode::decode(self.canonical_word).ok();
        let operand_mask = self
            .operands
            .iter()
            .fold(0, |mask, field| mask | field.mask);
        (0..u32::BITS)
            .filter_map(|bit| {
                if operand_mask & (1u32 << bit) != 0 {
                    return None;
                }
                let candidate = self.canonical_word ^ (1u32 << bit);
                (crate::decode::decode(candidate).ok() == original).then_some(candidate)
            })
            .collect()
    }

    /// Produces valid same-kind words by clearing one encoded operand bit.
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
                (exact_kind(candidate) == Some(self.kind)).then_some(candidate)
            })
            .collect()
    }
}

/// Lists every SPU instruction recipe in kind order.
pub fn generation_descriptors() -> Vec<SpuGenerationDescriptor> {
    build_generation_descriptors()
}

/// Finds the generation recipe for a decoded word.
pub fn generation_descriptor(raw: u32) -> Option<SpuGenerationDescriptor> {
    let instruction = crate::decode::decode(raw).ok()?;
    let contract = instruction.fuzz_descriptor();
    Some(SpuGenerationDescriptor {
        kind: contract.kind,
        form: contract.form,
        sequence_flow: sequence_flow(contract.kind, contract.outcomes),
        channel_values: channel_values(contract.kind),
        canonical_word: raw,
        operands: operand_fields(raw, instruction, contract),
    })
}

const NO_EFFECTS: &[EffectKind] = &[];
const RDCH_EFFECTS: &[EffectKind] = &[EffectKind::MailboxReceiveAttempt];
const WRCH_EFFECTS: &[EffectKind] = &[EffectKind::DmaEnqueue, EffectKind::ConditionalStore];
// The fuzz engine rejects outcomes outside these executor-derived sets.
const CONTINUE: &[SpuOutcomeClass] = &[SpuOutcomeClass::Continue];
const FAULT: &[SpuOutcomeClass] = &[SpuOutcomeClass::Fault];
const YIELD: &[SpuOutcomeClass] = &[SpuOutcomeClass::Yield];
const CONTINUE_OR_YIELD: &[SpuOutcomeClass] = &[SpuOutcomeClass::Continue, SpuOutcomeClass::Yield];
const LOAD_STORE: &[SpuOutcomeClass] = &[SpuOutcomeClass::Continue, SpuOutcomeClass::Fault];
const CONDITIONAL_BRANCH: &[SpuOutcomeClass] =
    &[SpuOutcomeClass::Continue, SpuOutcomeClass::Branch];
const UNCONDITIONAL_BRANCH: &[SpuOutcomeClass] = &[SpuOutcomeClass::Branch];
const WRCH: &[SpuOutcomeClass] = &[
    SpuOutcomeClass::Continue,
    SpuOutcomeClass::Yield,
    SpuOutcomeClass::MemoryRead,
    SpuOutcomeClass::Fault,
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
            relations: RELATIONS,
            decoded_execution_supported: execution_supported(*self),
            state_input: state_input(*self),
        }
    }
}

fn build_generation_descriptors() -> Vec<SpuGenerationDescriptor> {
    let mut words = BTreeMap::new();
    // Scanning every upper 18-bit value covers each opcode family.
    // Zero in the low 14 bits supplies canonical register values.
    for upper in 0..(1u32 << 18) {
        let raw = upper << 14;
        let Ok(instruction) = crate::decode::decode(raw) else {
            continue;
        };
        words
            .entry(SpuInstructionKind::from(instruction))
            .or_insert(raw);
    }
    words
        .into_iter()
        .filter_map(|(kind, canonical_word)| {
            let instruction = crate::decode::decode(canonical_word).ok()?;
            let contract = instruction.fuzz_descriptor();
            Some(SpuGenerationDescriptor {
                kind,
                form: contract.form,
                sequence_flow: sequence_flow(kind, contract.outcomes),
                channel_values: channel_values(kind),
                canonical_word,
                operands: operand_fields(canonical_word, instruction, contract),
            })
        })
        .collect()
}

fn operand_fields(
    raw: u32,
    instruction: SpuInstruction,
    descriptor: SpuFuzzDescriptor,
) -> Vec<SpuOperandField> {
    field_candidates(descriptor.kind, descriptor.form)
        .into_iter()
        .filter_map(|(mask, class)| {
            let decoded_field_is_ignored = matches!(
                descriptor.kind,
                SpuInstructionKind::Nop
                    | SpuInstructionKind::Bi
                    | SpuInstructionKind::Bisl
                    | SpuInstructionKind::Biz
                    | SpuInstructionKind::Binz
                    | SpuInstructionKind::Bihz
                    | SpuInstructionKind::Bihnz
                    | SpuInstructionKind::Hbr
                    | SpuInstructionKind::Hbra
                    | SpuInstructionKind::Hbrr
                    | SpuInstructionKind::Sync
                    | SpuInstructionKind::Heq
            );
            let active = if decoded_field_is_ignored {
                mask
            } else {
                active_bits(raw, instruction, descriptor.kind) & mask
            };
            (active != 0).then_some(SpuOperandField {
                class,
                mask: active,
            })
        })
        .collect()
}

fn active_bits(raw: u32, instruction: SpuInstruction, kind: SpuInstructionKind) -> u32 {
    (0..u32::BITS).fold(0, |mask, bit| {
        let candidate = raw ^ (1u32 << bit);
        match crate::decode::decode(candidate) {
            Ok(decoded) if decoded != instruction && SpuInstructionKind::from(decoded) == kind => {
                mask | (1u32 << bit)
            }
            _ => mask,
        }
    })
}

fn field_candidates(
    kind: SpuInstructionKind,
    form: SpuEncodingForm,
) -> Vec<(u32, SpuOperandClass)> {
    use SpuEncodingForm as F;
    use SpuInstructionKind as K;
    use SpuOperandClass as C;
    match form {
        F::Rrr => vec![
            (0x0000_007f, C::Register),
            (0x0000_3f80, C::Register),
            (0x001f_c000, C::Register),
        ],
        F::Rrrr => vec![
            (0x0000_007f, C::Register),
            (0x0000_3f80, C::Register),
            (0x001f_c000, C::Register),
            (0x0fe0_0000, C::Register),
        ],
        F::Ri7 => vec![
            (0x0000_007f, C::Register),
            (0x0000_3f80, C::Register),
            (0x001f_c000, C::Immediate),
        ],
        F::Ri10 => vec![
            (0x0000_007f, C::Register),
            (0x0000_3f80, C::Register),
            (0x00ff_c000, C::Immediate),
        ],
        F::Ri16 => vec![(0x0000_007f, C::Register), (0x007f_ff80, C::Immediate)],
        F::Ri18 => vec![(0x0000_007f, C::Register), (0x01ff_ff80, C::Immediate)],
        F::Channel => vec![(0x0000_007f, C::Register), (0x0000_3f80, C::Channel)],
        F::Branch => match kind {
            K::Br => vec![(0x007f_ff80, C::Immediate)],
            K::Brsl | K::Brz | K::Brnz | K::Brhnz | K::Brhz => {
                vec![(0x0000_007f, C::Register), (0x007f_ff80, C::Immediate)]
            }
            // [SPU-ISA p:178 s:7 Compare, Branch, and Halt Instructions] BI encodes E and D options.
            K::Bi => vec![
                (0x0000_3f80, C::Register),
                (0x0004_0000, C::Flag),
                (0x0008_0000, C::Flag),
            ],
            K::Bisl | K::Biz | K::Binz | K::Bihz | K::Bihnz => {
                // [SPU-ISA p:181 s:7 Compare, Branch, and Halt Instructions] BISL encodes E and D options.
                // [SPU-ISA p:186 s:7 Compare, Branch, and Halt Instructions] BIZ encodes E and D options.
                // [SPU-ISA p:187 s:7 Compare, Branch, and Halt Instructions] BINZ encodes E and D options.
                // [SPU-ISA p:188 s:7 Compare, Branch, and Halt Instructions] BIHZ encodes E and D options.
                // [SPU-ISA p:189 s:7 Compare, Branch, and Halt Instructions] BIHNZ encodes E and D options.
                vec![
                    (0x0000_007f, C::Register),
                    (0x0000_3f80, C::Register),
                    (0x0004_0000, C::Flag),
                    (0x0008_0000, C::Flag),
                ]
            }
            _ => Vec::new(),
        },
        F::Control => match kind {
            // [SPU-ISA p:238 s:10 Control Instructions] STOP carries one 14-bit signal operand.
            K::Stop => vec![(0x0000_3fff, C::Immediate)],
            // [SPU-ISA p:241 s:10 Control Instructions] NOP carries an RT false target.
            K::Nop => vec![(0x0000_007f, C::Register)],
            // [SPU-ISA p:150 s:7 Compare, Branch, and Halt Instructions] HEQ encodes RB, RA, and a false RT.
            K::Heq => vec![
                (0x0000_007f, C::Register),
                (0x0000_3f80, C::Register),
                (0x001f_c000, C::Register),
            ],
            // [SPU-ISA p:192 s:8 Hint-for-Branch Instructions] HBR encodes RO around RA and a P option.
            K::Hbr => vec![
                (0x0000_c07f, C::Immediate),
                (0x0000_3f80, C::Register),
                (0x0010_0000, C::Flag),
            ],
            // [SPU-ISA p:193 s:8 Hint-for-Branch Instructions] HBRA splits RO around its I16 field.
            K::Hbra => vec![(0x0180_007f, C::Immediate), (0x007f_ff80, C::Immediate)],
            // [SPU-ISA p:194 s:8 Hint-for-Branch Instructions] HBRR uses the HBRA field layout.
            K::Hbrr => vec![(0x0180_007f, C::Immediate), (0x007f_ff80, C::Immediate)],
            // [SPU-ISA p:242 s:10 Control Instructions] SYNC's C bit is an encoding option.
            K::Sync => vec![(0x0010_0000, C::Flag)],
            _ => Vec::new(),
        },
    }
}

fn exact_kind(raw: u32) -> Option<SpuInstructionKind> {
    crate::decode::decode(raw)
        .ok()
        .map(SpuInstructionKind::from)
}

/// Tests a decoded word for undefined operand combinations.
pub fn encoding_has_undefined_operands(raw: u32) -> bool {
    exact_kind(raw).is_some_and(|kind| !operand_combination_is_valid(kind, raw))
}

/// Tests whether the executor supports a decoded word.
pub fn encoding_execution_is_supported(raw: u32) -> bool {
    let Ok(instruction) = crate::decode::decode(raw) else {
        return false;
    };
    if !execution_supported(instruction) {
        return false;
    }
    let kind = SpuInstructionKind::from(instruction);
    let controls_interrupts = matches!(
        kind,
        SpuInstructionKind::Bi
            | SpuInstructionKind::Bisl
            | SpuInstructionKind::Biz
            | SpuInstructionKind::Binz
            | SpuInstructionKind::Bihz
            | SpuInstructionKind::Bihnz
    ) && raw & 0x000c_0000 != 0;
    // [SPU-ISA p:178 s:7 Compare, Branch, and Halt Instructions] BI's E and D
    // options replace interrupt-enable state, which the executor does not model.
    // [SPU-ISA p:181 s:7 Compare, Branch, and Halt Instructions] BISL has the
    // same interrupt-control options.
    // [SPU-ISA p:186 s:7 Compare, Branch, and Halt Instructions] BIZ has the
    // same interrupt-control options.
    // [SPU-ISA p:187 s:7 Compare, Branch, and Halt Instructions] BINZ has the
    // same interrupt-control options.
    // [SPU-ISA p:188 s:7 Compare, Branch, and Halt Instructions] BIHZ has the
    // same interrupt-control options.
    // [SPU-ISA p:189 s:7 Compare, Branch, and Halt Instructions] BIHNZ has the
    // same interrupt-control options.
    !controls_interrupts
}

fn sequence_flow(kind: SpuInstructionKind, outcomes: &[SpuOutcomeClass]) -> SpuSequenceFlow {
    // [SPU-ISA p:150 s:7 Compare, Branch, and Halt Instructions] HEQ can stop
    // execution when its two source values compare equal.
    if kind == SpuInstructionKind::Heq {
        return SpuSequenceFlow::StateDependent;
    }
    if outcomes.contains(&SpuOutcomeClass::Branch) {
        SpuSequenceFlow::ControlTransfer
    } else if outcomes == FAULT || outcomes == YIELD {
        SpuSequenceFlow::Terminal
    } else if outcomes != CONTINUE {
        SpuSequenceFlow::StateDependent
    } else {
        SpuSequenceFlow::Linear
    }
}

const MFC_COMMAND_INPUTS: &[u32] = &[
    spu::MFC_PUT,
    spu::MFC_GET,
    spu::MFC_GETLLAR,
    spu::MFC_PUTLLC,
];
const MFC_TAG_UPDATE_INPUTS: &[u32] = &[
    spu::MFC_TAG_UPDATE_IMMEDIATE,
    spu::MFC_TAG_UPDATE_ANY,
    spu::MFC_TAG_UPDATE_ALL,
];

fn state_input(instruction: SpuInstruction) -> Option<SpuStateInput> {
    match instruction {
        SpuInstruction::Wrch {
            channel: spu::MFC_CMD,
            rt,
        } => Some(SpuStateInput {
            register: rt,
            values: MFC_COMMAND_INPUTS,
            preferred: Some(spu::MFC_PUTLLC),
        }),
        SpuInstruction::Wrch {
            channel: spu::MFC_WR_TAG_UPDATE,
            rt,
        } => Some(SpuStateInput {
            register: rt,
            values: MFC_TAG_UPDATE_INPUTS,
            preferred: Some(spu::MFC_TAG_UPDATE_IMMEDIATE),
        }),
        _ => None,
    }
}

fn execution_supported(instruction: SpuInstruction) -> bool {
    match instruction {
        SpuInstruction::Rdch { channel, .. } => RDCH_CHANNELS.contains(&u32::from(channel)),
        SpuInstruction::Wrch { channel, .. } => WRCH_CHANNELS.contains(&u32::from(channel)),
        SpuInstruction::Rchcnt { channel, .. } => RCHCNT_CHANNELS.contains(&u32::from(channel)),
        // [SPU-ISA p:150 s:7 Compare, Branch, and Halt Instructions] HEQ can
        // stop execution, but the current instruction representation retains
        // neither source register needed to decide that outcome.
        SpuInstruction::Heq => false,
        // [SPU-ISA p:238 s:10 Control Instructions] STOP signals its 14-bit
        // value to the external environment, while the executor only records
        // that the unit finished.
        SpuInstruction::Stop { .. } => false,
        _ => true,
    }
}

const RDCH_CHANNELS: &[u32] = &[
    spu::MFC_RD_TAG_STAT as u32,
    spu::MFC_RD_ATOMIC_STAT as u32,
    spu::SPU_RD_IN_MBOX as u32,
    spu::SPU_RD_MACH_STAT as u32,
];
// [CBE-Handbook p:463 s:17.12 SPU Mailbox Channels] Outbound mailbox writes
// send guest-visible messages, so they stay unsupported until execution emits them.
const WRCH_CHANNELS: &[u32] = &[
    spu::MFC_LSA as u32,
    spu::MFC_EAH as u32,
    spu::MFC_EAL as u32,
    spu::MFC_SIZE as u32,
    spu::MFC_TAG_ID as u32,
    spu::MFC_CMD as u32,
    spu::MFC_WR_TAG_MASK as u32,
    spu::MFC_WR_TAG_UPDATE as u32,
];
const RCHCNT_CHANNELS: &[u32] = &[spu::SPU_RD_MACH_STAT as u32];

fn channel_values(kind: SpuInstructionKind) -> &'static [u32] {
    match kind {
        SpuInstructionKind::Rdch => RDCH_CHANNELS,
        SpuInstructionKind::Wrch => WRCH_CHANNELS,
        SpuInstructionKind::Rchcnt => RCHCNT_CHANNELS,
        _ => &[],
    }
}

fn operand_combination_is_valid(kind: SpuInstructionKind, word: u32) -> bool {
    use SpuInstructionKind as K;
    match kind {
        // [SPU-ISA p:178 s:7 Compare, Branch, and Halt Instructions] BI reserves E and D set together.
        K::Bi => word & 0x000c_0000 != 0x000c_0000,
        // [SPU-ISA p:181 s:7 Compare, Branch, and Halt Instructions] BISL reserves E and D set together.
        // [SPU-ISA p:186 s:7 Compare, Branch, and Halt Instructions] BIZ reserves E and D set together.
        // [SPU-ISA p:187 s:7 Compare, Branch, and Halt Instructions] BINZ reserves E and D set together.
        // [SPU-ISA p:188 s:7 Compare, Branch, and Halt Instructions] BIHZ reserves E and D set together.
        // [SPU-ISA p:189 s:7 Compare, Branch, and Halt Instructions] BIHNZ reserves E and D set together.
        K::Bisl | K::Biz | K::Binz | K::Bihz | K::Bihnz => word & 0x000c_0000 != 0x000c_0000,
        // [SPU-ISA p:192 s:8 Hint-for-Branch Instructions] P requires the split RO field to be zero.
        K::Hbr => word & 0x0010_0000 == 0 || word & 0x0000_c07f == 0,
        _ => true,
    }
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

#[cfg(test)]
fn expected_generation_kinds() -> BTreeSet<SpuInstructionKind> {
    SpuInstructionKind::VARIANTS.iter().copied().collect()
}

fn effect_and_outcome(
    instruction: &SpuInstruction,
) -> (&'static [EffectKind], &'static [SpuOutcomeClass]) {
    match *instruction {
        SpuInstruction::Lqd { .. }
        | SpuInstruction::Lqx { .. }
        | SpuInstruction::Lqa { .. }
        | SpuInstruction::Lqr { .. }
        | SpuInstruction::Stqd { .. }
        | SpuInstruction::Stqx { .. }
        | SpuInstruction::Stqa { .. }
        | SpuInstruction::Stqr { .. } => (NO_EFFECTS, LOAD_STORE),
        SpuInstruction::Rdch {
            channel: spu::MFC_RD_TAG_STAT,
            ..
        } => (NO_EFFECTS, CONTINUE_OR_YIELD),
        SpuInstruction::Rdch {
            channel: spu::SPU_RD_IN_MBOX,
            ..
        } => (RDCH_EFFECTS, YIELD),
        SpuInstruction::Rdch {
            channel: spu::MFC_RD_ATOMIC_STAT | spu::SPU_RD_MACH_STAT,
            ..
        } => (NO_EFFECTS, CONTINUE),
        SpuInstruction::Rdch { .. } => (NO_EFFECTS, FAULT),
        SpuInstruction::Wrch {
            channel: spu::MFC_CMD,
            ..
        } => (WRCH_EFFECTS, WRCH),
        SpuInstruction::Wrch {
            channel:
                spu::MFC_LSA
                | spu::MFC_EAH
                | spu::MFC_EAL
                | spu::MFC_SIZE
                | spu::MFC_TAG_ID
                | spu::MFC_WR_TAG_MASK
                | spu::MFC_WR_TAG_UPDATE
                | spu::SPU_WR_OUT_MBOX,
            ..
        } => (NO_EFFECTS, CONTINUE),
        SpuInstruction::Wrch { .. } => (NO_EFFECTS, FAULT),
        SpuInstruction::Rchcnt {
            channel: spu::SPU_RD_MACH_STAT,
            ..
        } => (NO_EFFECTS, CONTINUE),
        SpuInstruction::Rchcnt { .. } => (NO_EFFECTS, FAULT),
        SpuInstruction::Br { .. }
        | SpuInstruction::Brsl { .. }
        | SpuInstruction::Bi { .. }
        | SpuInstruction::Bisl { .. } => (NO_EFFECTS, UNCONDITIONAL_BRANCH),
        SpuInstruction::Brz { .. }
        | SpuInstruction::Brnz { .. }
        | SpuInstruction::Brhnz { .. }
        | SpuInstruction::Brhz { .. }
        | SpuInstruction::Biz { .. }
        | SpuInstruction::Binz { .. }
        | SpuInstruction::Bihz { .. }
        | SpuInstruction::Bihnz { .. } => (NO_EFFECTS, CONDITIONAL_BRANCH),
        SpuInstruction::Stop { .. } => (NO_EFFECTS, YIELD),
        _ => (NO_EFFECTS, CONTINUE),
    }
}

/// Produces a simpler encoding when it keeps the instruction kind.
pub fn simplify_instruction_bit(raw: u32, bit_index: u32) -> Option<u32> {
    let bit = 1u32.checked_shl(bit_index)?;
    (raw & bit != 0).then_some(())?;
    let instruction = crate::decode::decode(raw).ok()?;
    let kind = SpuInstructionKind::from(instruction);
    let candidate = raw & !bit;
    let decoded = crate::decode::decode(candidate).ok()?;
    (SpuInstructionKind::from(decoded) == kind).then_some(candidate)
}

/// Produces valid same-kind candidates for fuzz-engine shrinking.
pub fn shrink_instruction(raw: u32) -> Vec<u32> {
    generation_descriptor(raw).map_or_else(Vec::new, |descriptor| descriptor.shrink(raw))
}

fn form_for_kind(kind: SpuInstructionKind) -> SpuEncodingForm {
    use SpuInstructionKind as K;
    match kind {
        K::Selb | K::Shufb => SpuEncodingForm::Rrrr,
        // [SPU-ISA p:29 s:2.3 Instruction Formats] RI10 carries I10 between its opcode and RA fields.
        K::Lqd | K::Stqd | K::Ai | K::Ori | K::Andi | K::Ceqi | K::Ceqbi | K::Cgti => {
            SpuEncodingForm::Ri10
        }
        K::Cbd
        | K::Chd
        | K::Cwd
        | K::Cdd
        | K::Shlqbyi
        | K::Rotqbyi
        | K::Rotqmbyi
        | K::Shli
        | K::Rotmi
        | K::Rotmai => SpuEncodingForm::Ri7,
        K::Lqa | K::Stqa | K::Lqr | K::Stqr | K::Il | K::Ilh | K::Ilhu | K::Iohl | K::Fsmbi => {
            SpuEncodingForm::Ri16
        }
        K::Ila => SpuEncodingForm::Ri18,
        K::Br
        | K::Brsl
        | K::Brz
        | K::Brnz
        | K::Bi
        | K::Bisl
        | K::Brhnz
        | K::Brhz
        | K::Biz
        | K::Binz
        | K::Bihz
        | K::Bihnz => SpuEncodingForm::Branch,
        K::Rdch | K::Wrch | K::Rchcnt => SpuEncodingForm::Channel,
        K::Nop | K::Lnop | K::Hbr | K::Hbra | K::Hbrr | K::Sync | K::Dsync | K::Heq | K::Stop => {
            SpuEncodingForm::Control
        }
        _ => SpuEncodingForm::Rrr,
    }
}

fn classify_kind(kind: SpuInstructionKind) {
    match kind {
        SpuInstructionKind::Lqd
        | SpuInstructionKind::Lqx
        | SpuInstructionKind::Lqa
        | SpuInstructionKind::Stqd
        | SpuInstructionKind::Stqx
        | SpuInstructionKind::Stqa
        | SpuInstructionKind::Lqr
        | SpuInstructionKind::Stqr
        | SpuInstructionKind::Il
        | SpuInstructionKind::Ila
        | SpuInstructionKind::Ilh
        | SpuInstructionKind::Ilhu
        | SpuInstructionKind::Iohl
        | SpuInstructionKind::Fsmbi
        | SpuInstructionKind::A
        | SpuInstructionKind::Ai
        | SpuInstructionKind::Sf
        | SpuInstructionKind::And
        | SpuInstructionKind::Or
        | SpuInstructionKind::Selb
        | SpuInstructionKind::Xsbh
        | SpuInstructionKind::Gb
        | SpuInstructionKind::Gbh
        | SpuInstructionKind::Ori
        | SpuInstructionKind::Nor
        | SpuInstructionKind::Andi
        | SpuInstructionKind::Shufb
        | SpuInstructionKind::Shlqbyi
        | SpuInstructionKind::Rotqby
        | SpuInstructionKind::Rotqbyi
        | SpuInstructionKind::Rotqmbyi
        | SpuInstructionKind::Shl
        | SpuInstructionKind::Shli
        | SpuInstructionKind::Rotmi
        | SpuInstructionKind::Rotmai
        | SpuInstructionKind::Cbd
        | SpuInstructionKind::Cbx
        | SpuInstructionKind::Chd
        | SpuInstructionKind::Chx
        | SpuInstructionKind::Cwd
        | SpuInstructionKind::Cwx
        | SpuInstructionKind::Cdd
        | SpuInstructionKind::Cdx
        | SpuInstructionKind::Ceq
        | SpuInstructionKind::Ceqi
        | SpuInstructionKind::Ceqbi
        | SpuInstructionKind::Cgti
        | SpuInstructionKind::Clgt
        | SpuInstructionKind::Br
        | SpuInstructionKind::Brsl
        | SpuInstructionKind::Brz
        | SpuInstructionKind::Brnz
        | SpuInstructionKind::Bi
        | SpuInstructionKind::Bisl
        | SpuInstructionKind::Brhnz
        | SpuInstructionKind::Brhz
        | SpuInstructionKind::Biz
        | SpuInstructionKind::Binz
        | SpuInstructionKind::Bihz
        | SpuInstructionKind::Bihnz
        | SpuInstructionKind::Rdch
        | SpuInstructionKind::Wrch
        | SpuInstructionKind::Rchcnt
        | SpuInstructionKind::Nop
        | SpuInstructionKind::Lnop
        | SpuInstructionKind::Hbr
        | SpuInstructionKind::Hbra
        | SpuInstructionKind::Hbrr
        | SpuInstructionKind::Sync
        | SpuInstructionKind::Dsync
        | SpuInstructionKind::Heq
        | SpuInstructionKind::Stop => {}
    }
}

#[cfg(test)]
#[path = "tests/fuzz_tests.rs"]
mod tests;
