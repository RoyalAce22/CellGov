//! The fuzz vocabulary: encoding forms, fuzz kinds, outcome and relation classes, and the descriptors.

use cellgov_effects::EffectKind;
use cellgov_exec::operand::{OperandClass, OperandField};

use crate::instruction::ops::{Fp59Op, Fp63Op, VaOp, VxOp};
use crate::instruction::PpuInstructionKind;

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
    // [Le2014 p:219 s:3.1] The comparison assumes deterministic semantics, where repeated executions on the same input yield the same result, and this relation checks that assumption.
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
pub type PpuOperandField = OperandField<PpuOperandClass>;

impl OperandClass for PpuOperandClass {
    const REGISTER: Self = Self::Register;
    const IMMEDIATE: Self = Self::Immediate;
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
