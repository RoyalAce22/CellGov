//! Typed eligibility and generation features for fuzz cases.

use std::collections::BTreeSet;

/// Whether a generated case can enter its selected semantic check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CaseEligibility {
    /// The case satisfies the selected check's preconditions.
    Eligible,
    /// CellGov does not model a precondition needed by the selected check.
    Unsupported,
    /// The architecture does not define the selected check for this case.
    Undefined,
}

/// Why a case received its eligibility classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EligibilityReason {
    /// An interpreter descriptor defines the local invariant check.
    InterpreterContract,
    /// A decoded raw word is eligible only for decoder-robustness checks.
    DecoderRobustness,
    /// Generated architectural state satisfies the descriptor's preconditions.
    StatePreconditions,
    /// The descriptor selects a defined architectural fault boundary.
    NamedFaultBoundary,
    /// Generated state could not satisfy a modeled precondition.
    UnmetStatePrecondition,
    /// The selected instruction or operand has no modeled execution semantics.
    UnmodeledExecution,
    /// The interpreter descriptor marks the case architecturally undefined.
    ArchitecturallyUndefined,
}

/// A semantic feature established while constructing a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CaseFeature {
    /// Two or more encoded operands select the same register.
    OperandAlias,
    /// An encoded operand uses a declared boundary value.
    OperandBoundary,
    /// Register state targets a valid mapped-memory or local-store region.
    MappedMemory,
    /// State carries a reservation aligned to the generated access region.
    Reservation,
    /// SPU channel state carries usable tag, mailbox, or transfer inputs.
    ChannelState,
    /// Consecutive instructions share a generated register dependency.
    DependencyChain,
    /// Sequence generation bounds a control transfer or stop.
    ControlledFlow,
    /// State or operands select a named architectural fault boundary.
    NamedFaultBoundary,
}

/// Records a case's eligibility and features before semantic comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseAssessment {
    /// Eligibility for the selected check.
    pub eligibility: CaseEligibility,
    /// Typed reasons for the classification.
    pub reasons: BTreeSet<EligibilityReason>,
    /// Semantic features established during generation.
    pub features: BTreeSet<CaseFeature>,
}

impl CaseAssessment {
    /// Starts a classification with one required reason.
    pub fn new(
        eligibility: CaseEligibility,
        reason: EligibilityReason,
        features: impl IntoIterator<Item = CaseFeature>,
    ) -> Self {
        Self {
            eligibility,
            reasons: BTreeSet::from([reason]),
            features: features.into_iter().collect(),
        }
    }

    /// Adds another reason without changing the eligibility class.
    pub fn with_reason(mut self, reason: EligibilityReason) -> Self {
        self.reasons.insert(reason);
        self
    }
}

#[cfg(test)]
#[path = "tests/case_tests.rs"]
mod tests;
