use super::*;

#[test]
fn a_new_assessment_holds_exactly_the_given_reason() {
    let assessment = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::InterpreterContract,
        [],
    );

    assert_eq!(assessment.eligibility, CaseEligibility::Eligible);
    assert_eq!(
        assessment.reasons,
        BTreeSet::from([EligibilityReason::InterpreterContract])
    );
    assert!(assessment.features.is_empty());
}

#[test]
fn features_are_deduplicated_into_declaration_order() {
    let assessment = CaseAssessment::new(
        CaseEligibility::Unsupported,
        EligibilityReason::UnmetStatePrecondition,
        [
            CaseFeature::MappedMemory,
            CaseFeature::OperandAlias,
            CaseFeature::MappedMemory,
        ],
    );

    assert_eq!(
        assessment.features.into_iter().collect::<Vec<_>>(),
        [CaseFeature::OperandAlias, CaseFeature::MappedMemory]
    );
}

#[test]
fn with_reason_adds_a_reason_and_keeps_the_eligibility() {
    let assessment = CaseAssessment::new(
        CaseEligibility::Undefined,
        EligibilityReason::ArchitecturallyUndefined,
        [],
    )
    .with_reason(EligibilityReason::InterpreterContract)
    .with_reason(EligibilityReason::ArchitecturallyUndefined);

    assert_eq!(assessment.eligibility, CaseEligibility::Undefined);
    assert_eq!(
        assessment.reasons.into_iter().collect::<Vec<_>>(),
        [
            EligibilityReason::InterpreterContract,
            EligibilityReason::ArchitecturallyUndefined,
        ]
    );
}

#[test]
fn assessments_compare_by_content_not_by_feature_order() {
    let left = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::StatePreconditions,
        [CaseFeature::Reservation, CaseFeature::MappedMemory],
    );
    let right = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::StatePreconditions,
        [CaseFeature::MappedMemory, CaseFeature::Reservation],
    );

    assert_eq!(left, right);
}

#[test]
fn eligibility_orders_eligible_before_unsupported_before_undefined() {
    let mut classes = [
        CaseEligibility::Undefined,
        CaseEligibility::Eligible,
        CaseEligibility::Unsupported,
    ];
    classes.sort();

    assert_eq!(
        classes,
        [
            CaseEligibility::Eligible,
            CaseEligibility::Unsupported,
            CaseEligibility::Undefined,
        ]
    );
}

#[test]
fn eligibility_reasons_order_by_declaration() {
    let declared = [
        EligibilityReason::InterpreterContract,
        EligibilityReason::DecoderRobustness,
        EligibilityReason::StatePreconditions,
        EligibilityReason::NamedFaultBoundary,
        EligibilityReason::UnmetStatePrecondition,
        EligibilityReason::UnmodeledExecution,
        EligibilityReason::ArchitecturallyUndefined,
    ];
    let mut reversed = declared;
    reversed.reverse();
    reversed.sort();

    assert_eq!(reversed, declared);
}

#[test]
fn case_features_order_by_declaration() {
    let declared = [
        CaseFeature::OperandAlias,
        CaseFeature::OperandBoundary,
        CaseFeature::MappedMemory,
        CaseFeature::Reservation,
        CaseFeature::ChannelState,
        CaseFeature::DependencyChain,
        CaseFeature::ControlledFlow,
        CaseFeature::NamedFaultBoundary,
    ];
    let mut reversed = declared;
    reversed.reverse();
    reversed.sort();

    assert_eq!(reversed, declared);
}
