//! The fuzz vocabulary: encoding forms, outcome and relation classes, operand fields and the descriptors.

use cellgov_effects::EffectKind;
use cellgov_exec::operand::{OperandClass, OperandField};

use crate::instruction::SpuInstructionKind;

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
    // [Le2014 p:219 s:3.1] The comparison assumes deterministic semantics, where repeated executions on the same input yield the same result, and this relation checks that assumption.
    Deterministic,
    /// NOP's false target field leaves the complete observation unchanged.
    NopFalseTarget,
    /// The high immediate bits do not affect a quadword byte rotation.
    RotateByteCountHighBit,
}

/// A partner word whose complete observation must match the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuMetamorphicCase {
    /// Selects the rule used to derive the partner word.
    pub relation: SpuMetamorphicRelation,
    /// Encodes the instruction to run from the original initial state.
    pub partner_word: u32,
}

/// Reason a partner word cannot be compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SpuRelationRefusal {
    /// The instruction does not declare the requested relation.
    #[error("SPU instruction does not declare relation {relation:?}")]
    Undeclared {
        /// Relation absent from the instruction descriptor.
        relation: SpuMetamorphicRelation,
    },
    /// No valid partner word exists for this encoding.
    #[error("SPU relation {relation:?} has no alternate encoding")]
    NoPartner {
        /// Relation without a valid partner word.
        relation: SpuMetamorphicRelation,
    },
    /// The original or partner word has undefined or unsupported behavior.
    #[error("SPU relation {relation:?} is undefined or unsupported")]
    Ineligible {
        /// Relation refused by the eligibility checks.
        relation: SpuMetamorphicRelation,
    },
    /// The partner word fails the relation's decoding condition.
    #[error("SPU relation {relation:?} produced an invalid partner")]
    InvalidPartner {
        /// Relation whose partner word failed the decoding check.
        relation: SpuMetamorphicRelation,
    },
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
pub type SpuOperandField = OperandField<SpuOperandClass>;

impl OperandClass for SpuOperandClass {
    const REGISTER: Self = Self::Register;
    const IMMEDIATE: Self = Self::Immediate;
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
    /// The sequence registry lacks a required decoded instruction kind.
    #[error("SPU sequence requires instruction kind {kind:?}")]
    MissingSequenceKind {
        /// Instruction needed for the sequence recipe.
        kind: SpuInstructionKind,
    },
}
