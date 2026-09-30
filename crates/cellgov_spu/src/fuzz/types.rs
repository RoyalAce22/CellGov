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
    /// Register plus 8-bit immediate.
    Ri8,
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
    /// The instruction stopped the SPU.
    Stop,
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
            crate::exec::SpuStepOutcome::Stop { .. } => Self::Stop,
        }
    }
}

/// Interpreter self-relation suitable for fuzz checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpuMetamorphicRelation {
    /// Identical inputs give identical outputs.
    ///
    /// [Le2014 p:219 s:3.1] The comparison assumes deterministic semantics, where repeated executions on the same input yield the same result, and this relation checks that assumption.
    Deterministic,
    /// Word bits the ISA marks ignored, or a false target, leave the complete
    /// observation unchanged.
    IgnoredField,
    /// Count bits a shift or rotate masks off leave the result unchanged.
    CountMasking,
    /// A shufb control byte changed within its class gives the same result byte.
    ShufbControlClass,
    /// An immediate form equals its register form with the extended
    /// immediate in every element of RB.
    ImmediateRegister,
    /// A swap of RA and RB of a symmetric operation leaves the observation
    /// unchanged.
    Commutative,
    /// A swap of the doublewords of every input of an element-wise operation
    /// swaps the doublewords of its result.
    SlotPermutation,
    /// A conditional branch goes where the opposite-sense branch goes on the
    /// mask a compare of the tested value against zero gives.
    CompareBranch,
}

/// A partner whose complete observation must match the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuMetamorphicCase {
    /// Selects the rule used to derive the partner.
    pub relation: SpuMetamorphicRelation,
    /// Encodes the instruction to run from the partner's initial state.
    pub partner_word: u32,
    /// The input registers the relation rewrites in the partner's initial
    /// state, each named once; all `None` when the partner runs from the
    /// original initial state.
    pub varied_inputs: [Option<SpuVariedInput>; 3],
    /// The result register whose doublewords
    /// [`SpuMetamorphicCase::settle_partner`] swaps back.
    pub permuted_output: Option<u8>,
}

/// One input register a state relation rewrites before the partner runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuVariedInput {
    /// The rewritten register.
    pub register: u8,
    /// How the partner's value derives from the original's.
    pub rewrite: SpuInputRewrite,
    /// True when the instruction does not write the register.
    ///
    /// [`SpuMetamorphicCase::settle_partner`] then copies the register in
    /// the partner's final state from the original's.
    pub restore: bool,
}

/// How a state relation rewrites one input register.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpuInputRewrite {
    /// Each shufb control byte moves to another byte of its class.
    ShufbControlClass,
    /// The register takes this value.
    Replace([u8; 16]),
    /// The set bits of this mask flip.
    Flip([u8; 16]),
    /// The tested field becomes all ones when it was zero and zero
    /// otherwise: the preferred word, or bytes 2:3 when `halfword`.
    ZeroCompare {
        /// Tests bytes 2:3 instead of the preferred word.
        halfword: bool,
    },
    /// The two doublewords trade places.
    SwapDoublewords,
}

/// Reason a relation has no partner to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SpuRelationRefusal {
    /// The instruction does not declare the requested relation.
    #[error("SPU instruction does not declare relation {relation:?}")]
    Undeclared {
        /// Relation absent from the instruction descriptor.
        relation: SpuMetamorphicRelation,
    },
    /// No valid partner exists for this encoding.
    #[error("SPU relation {relation:?} has no partner for this encoding")]
    NoPartner {
        /// Relation without a valid partner.
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
