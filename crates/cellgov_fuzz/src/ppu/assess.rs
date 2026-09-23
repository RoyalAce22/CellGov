//! Eligibility of one PPU instruction or sequence case for the contract checks.

use std::collections::BTreeSet;

use cellgov_ppu::exec::ExecuteVerdict;
use cellgov_ppu::instruction::fuzz::PpuOutcomeClass;
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::state::PpuState;

use crate::case::{CaseAssessment, CaseEligibility, CaseFeature, EligibilityReason};
use crate::GenerationStrategy;

pub(super) fn assess_instruction_case(
    strategy: GenerationStrategy,
    instruction: &PpuInstruction,
    initial: &PpuState,
    descriptor: cellgov_ppu::instruction::fuzz::PpuFuzzDescriptor,
    verdict: &ExecuteVerdict,
    mut features: BTreeSet<CaseFeature>,
) -> CaseAssessment {
    if instruction.fuzz_case_is_architecturally_undefined(initial) {
        return CaseAssessment::new(
            CaseEligibility::Undefined,
            EligibilityReason::ArchitecturallyUndefined,
            features,
        );
    }
    let outcome = PpuOutcomeClass::from_verdict(verdict);
    if strategy == GenerationStrategy::Structured
        && matches!(
            outcome,
            PpuOutcomeClass::MemoryFault | PpuOutcomeClass::Fault
        )
        && descriptor.outcomes.contains(&PpuOutcomeClass::Continue)
        && descriptor.outcomes.contains(&outcome)
    {
        if outcome == PpuOutcomeClass::MemoryFault {
            features.remove(&CaseFeature::MappedMemory);
            features.remove(&CaseFeature::Reservation);
        }
        return CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmetStatePrecondition,
            features,
        );
    }
    if strategy == GenerationStrategy::Structured && descriptor.outcomes == [PpuOutcomeClass::Fault]
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
    features: BTreeSet<CaseFeature>,
) -> CaseAssessment {
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
