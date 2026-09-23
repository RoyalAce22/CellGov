//! Eligibility of one SPU instruction or sequence case for the contract checks.

use std::collections::BTreeSet;

use cellgov_spu::exec::SpuStepOutcome;
use cellgov_spu::fuzz::{SpuFuzzDescriptor, SpuOutcomeClass};

use crate::case::{CaseAssessment, CaseEligibility, CaseFeature, EligibilityReason};
use crate::GenerationStrategy;

// [Yang2011 p:2 s:2.2] A case whose meaning the architecture leaves undefined cannot expose a wrong result, so it enters no check.
pub(super) fn assess_instruction_case(
    strategy: GenerationStrategy,
    has_undefined_operands: bool,
    execution_is_supported: bool,
    descriptor: SpuFuzzDescriptor,
    outcome: &SpuStepOutcome,
    mut features: BTreeSet<CaseFeature>,
) -> CaseAssessment {
    if has_undefined_operands {
        return CaseAssessment::new(
            CaseEligibility::Undefined,
            EligibilityReason::ArchitecturallyUndefined,
            features,
        );
    }
    if !execution_is_supported {
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmodeledExecution,
            features,
        );
    }
    let outcome = SpuOutcomeClass::from_outcome(outcome);
    if strategy == GenerationStrategy::Structured
        && outcome == SpuOutcomeClass::Fault
        && descriptor.outcomes.contains(&SpuOutcomeClass::Continue)
        && descriptor.outcomes.contains(&outcome)
    {
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmetStatePrecondition,
            features,
        );
    }
    if strategy == GenerationStrategy::Structured && descriptor.outcomes == [SpuOutcomeClass::Fault]
    {
        features.insert(CaseFeature::NamedFaultBoundary);
        return CaseAssessment::new(
            CaseEligibility::Eligible,
            EligibilityReason::NamedFaultBoundary,
            features,
        )
        .with_reason(EligibilityReason::InterpreterContract);
    }
    let reason = match strategy {
        GenerationStrategy::Structured => EligibilityReason::StatePreconditions,
        GenerationStrategy::RawWords => EligibilityReason::DecoderRobustness,
    };
    CaseAssessment::new(CaseEligibility::Eligible, reason, features)
        .with_reason(EligibilityReason::InterpreterContract)
}

pub(super) fn assess_sequence_case(
    strategy: GenerationStrategy,
    executed_depth: u64,
    has_undefined_operands: bool,
    has_unmodeled_execution: bool,
    features: BTreeSet<CaseFeature>,
) -> CaseAssessment {
    if has_undefined_operands {
        return CaseAssessment::new(
            CaseEligibility::Undefined,
            EligibilityReason::ArchitecturallyUndefined,
            features,
        );
    }
    if has_unmodeled_execution {
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmodeledExecution,
            features,
        );
    }
    if strategy == GenerationStrategy::Structured && executed_depth == 0 {
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmetStatePrecondition,
            features,
        );
    }
    let reason = match strategy {
        GenerationStrategy::Structured => EligibilityReason::StatePreconditions,
        GenerationStrategy::RawWords => EligibilityReason::DecoderRobustness,
    };
    CaseAssessment::new(CaseEligibility::Eligible, reason, features)
        .with_reason(EligibilityReason::InterpreterContract)
}

#[cfg(test)]
#[path = "tests/assess_tests.rs"]
mod tests;
