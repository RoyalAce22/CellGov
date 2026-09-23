use super::*;

#[test]
fn an_unexpected_fault_remains_eligible_for_contract_checks() {
    let descriptor = cellgov_spu::instruction::SpuInstruction::Ai {
        rt: 0,
        ra: 1,
        imm: 0,
    }
    .fuzz_descriptor();
    assert_eq!(descriptor.outcomes, &[SpuOutcomeClass::Continue]);

    let assessment = assess_instruction_case(
        GenerationStrategy::Structured,
        false,
        true,
        descriptor,
        &SpuStepOutcome::Fault(cellgov_spu::exec::SpuFault::LsOutOfRange(0)),
        BTreeSet::new(),
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Eligible);
}

#[test]
fn a_structured_sequence_that_executed_nothing_is_an_unmet_precondition() {
    let structured = assess_sequence_case(
        GenerationStrategy::Structured,
        0,
        false,
        false,
        BTreeSet::new(),
    );
    assert_eq!(structured.eligibility, CaseEligibility::Unsupported);
    assert!(structured
        .reasons
        .contains(&EligibilityReason::UnmetStatePrecondition));

    let raw = assess_sequence_case(
        GenerationStrategy::RawWords,
        0,
        false,
        false,
        BTreeSet::new(),
    );
    assert_eq!(raw.eligibility, CaseEligibility::Eligible);

    let undefined =
        assess_sequence_case(GenerationStrategy::RawWords, 3, true, true, BTreeSet::new());
    assert_eq!(undefined.eligibility, CaseEligibility::Undefined);
}
