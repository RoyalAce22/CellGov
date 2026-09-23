use super::*;

use cellgov_ppu::instruction::PpuInstructionKind;
use serde_json::json;

use crate::{ConfigurationError, CrossReferenceAsymmetry, OperandAliasClass, StateTransitionClass};

const SPU_NOP: InstructionIdentity = InstructionIdentity::Spu(SpuInstructionKind::Nop);
const PPU_ORI: InstructionIdentity =
    InstructionIdentity::Ppu(PpuFuzzKind::Ordinary(PpuInstructionKind::Ori));

fn report(max_findings: usize) -> FuzzReport {
    FuzzReport::new(
        FuzzTarget::PpuInstruction,
        7,
        GenerationStrategy::Structured,
        RetentionConfig::default(),
        max_findings,
        1,
    )
}

fn finding(kind: FindingKind, case_index: u64) -> Finding {
    Finding {
        fingerprint: SemanticFingerprint {
            target: FuzzTarget::PpuInstruction,
            instruction_kind: None,
            check: CheckIdentity::LegalOutcome,
            divergence: DivergenceClass::Outcome,
            outcome: None,
            effect: None,
        },
        kind,
        replay: ReplayCoordinates::new(
            FuzzTarget::PpuInstruction,
            GenerationStrategy::Structured,
            7,
            case_index,
            1,
        ),
        original_words: vec![0x6000_0000],
        observation: None,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: None,
    }
}

fn observation(depth: u64) -> SemanticObservation {
    SemanticObservation {
        first_instruction_kind: Some(SPU_NOP),
        instruction_kinds: BTreeSet::from([SPU_NOP]),
        operands: OperandAliasClass::Ordinary,
        eligibility: CaseEligibility::Eligible,
        outcome: Some(OutcomeIdentity::SpuContinue),
        state_transition: StateTransitionClass::Unchanged,
        effects: BTreeSet::new(),
        boundaries: BTreeSet::new(),
        sequence_depth: depth,
        asymmetry: CrossReferenceAsymmetry::None,
    }
}

fn eligible(reason: EligibilityReason) -> CaseAssessment {
    CaseAssessment::new(CaseEligibility::Eligible, reason, [])
}

fn overflow(counter: &'static str) -> Result<(), InvariantError> {
    Err(InvariantError::CounterOverflow { counter })
}

fn outcome_for(
    kinds: &[FindingKind],
    eligible: u64,
    unsupported: u64,
    undefined: u64,
) -> RunOutcome {
    let mut report = report(0);
    for kind in kinds {
        report.finding_counts.insert(*kind, 1);
    }
    report.eligible_cases = eligible;
    report.unsupported_cases = unsupported;
    report.undefined_cases = undefined;
    FuzzRun::completed(report).outcome
}

#[test]
fn a_new_report_carries_its_identity_and_zeroed_counters() {
    let retention = RetentionConfig {
        capacity: 3,
        per_kind_capacity: 1,
        ..RetentionConfig::default()
    };
    let report = FuzzReport::new(
        FuzzTarget::SpuSequence,
        11,
        GenerationStrategy::RawWords,
        retention,
        2,
        9,
    );

    assert_eq!(report.target, FuzzTarget::SpuSequence);
    assert_eq!(report.seed, 11);
    assert_eq!(report.strategy, GenerationStrategy::RawWords);
    assert_eq!(report.sequence_words, 9);
    assert_eq!(report.retained_cases.config(), retention);
    assert_eq!(
        (
            report.cases,
            report.decoded,
            report.eligible_cases,
            report.unsupported_cases,
            report.undefined_cases,
            report.executed_steps,
            report.max_executed_depth,
        ),
        (0, 0, 0, 0, 0, 0, 0)
    );
    assert_eq!(report.distribution, CampaignDistribution::default());
    assert!(report.instruction_kinds.is_empty());
    assert!(report.findings.is_empty());
    assert!(report.is_clean());
    assert_eq!(report.eligibility_rate(), None);
    assert_eq!(
        report.trial_metrics(),
        TrialMetrics {
            seed: 11,
            attempted: 0,
            eligible: 0,
            executed: 0,
            retained: 0,
        }
    );
}

#[test]
fn considered_counts_cases_and_attempts_together() {
    let mut report = report(0);

    assert_eq!(report.considered(), Ok(()));
    assert_eq!(report.considered(), Ok(()));
    assert_eq!(report.cases, 2);
    assert_eq!(report.distribution.attempted, 2);
}

#[test]
fn considered_refuses_a_saturated_case_counter() {
    let mut report = report(0);
    report.cases = u64::MAX;

    assert_eq!(report.considered(), overflow("cases"));
}

#[test]
fn considered_refuses_a_saturated_attempted_distribution() {
    let mut report = report(0);
    report.distribution.attempted = u64::MAX;

    assert_eq!(report.considered(), overflow("attempted distribution"));
}

#[test]
fn reached_counts_decodes_and_collects_distinct_kinds() {
    let mut report = report(0);

    assert_eq!(report.reached(SPU_NOP), Ok(()));
    assert_eq!(report.reached(SPU_NOP), Ok(()));
    assert_eq!(report.reached(PPU_ORI), Ok(()));
    assert_eq!(report.decoded, 3);
    assert_eq!(report.instruction_kinds, BTreeSet::from([PPU_ORI, SPU_NOP]));
}

#[test]
fn reached_refuses_a_saturated_decode_counter_without_recording_the_kind() {
    let mut report = report(0);
    report.decoded = u64::MAX;

    assert_eq!(report.reached(SPU_NOP), overflow("decoded"));
    assert!(report.instruction_kinds.is_empty());
}

#[test]
fn reached_many_adds_the_count_and_every_kind() {
    let mut report = report(0);

    assert_eq!(report.reached_many(u64::MAX, [SPU_NOP, PPU_ORI]), Ok(()));
    assert_eq!(report.decoded, u64::MAX);
    assert_eq!(report.instruction_kinds, BTreeSet::from([PPU_ORI, SPU_NOP]));
}

#[test]
fn reached_many_with_a_zero_count_still_records_kinds() {
    let mut report = report(0);

    assert_eq!(report.reached_many(0, [SPU_NOP]), Ok(()));
    assert_eq!(report.decoded, 0);
    assert_eq!(report.instruction_kinds, BTreeSet::from([SPU_NOP]));
}

#[test]
fn reached_many_refuses_a_count_that_overflows_the_decode_counter() {
    let mut report = report(0);
    report.decoded = 1;

    assert_eq!(report.reached_many(u64::MAX, []), overflow("decoded"));
    assert_eq!(report.decoded, 1);
}

#[test]
fn assessed_routes_each_eligibility_class_to_its_own_counter() {
    let mut report = report(0);
    let unsupported = CaseAssessment::new(
        CaseEligibility::Unsupported,
        EligibilityReason::UnmetStatePrecondition,
        [],
    );
    let undefined = CaseAssessment::new(
        CaseEligibility::Undefined,
        EligibilityReason::ArchitecturallyUndefined,
        [],
    );

    assert_eq!(
        report.assessed(&eligible(EligibilityReason::InterpreterContract)),
        Ok(())
    );
    assert_eq!(
        report.assessed(&eligible(EligibilityReason::InterpreterContract)),
        Ok(())
    );
    assert_eq!(report.assessed(&unsupported), Ok(()));
    assert_eq!(report.assessed(&undefined), Ok(()));
    assert_eq!(
        (
            report.eligible_cases,
            report.unsupported_cases,
            report.undefined_cases,
        ),
        (2, 1, 1)
    );
    assert_eq!(
        report.distribution.eligibility,
        BTreeMap::from([
            (CaseEligibility::Eligible, 2),
            (CaseEligibility::Unsupported, 1),
            (CaseEligibility::Undefined, 1),
        ])
    );
    assert_eq!(report.eligibility_rate(), Some((2, 4)));
}

#[test]
fn assessed_counts_every_reason_and_feature_of_the_case() {
    let mut report = report(0);
    let assessment = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::StatePreconditions,
        [CaseFeature::MappedMemory, CaseFeature::Reservation],
    )
    .with_reason(EligibilityReason::InterpreterContract);

    assert_eq!(report.assessed(&assessment), Ok(()));
    assert_eq!(report.assessed(&assessment), Ok(()));
    assert_eq!(
        report.eligibility_reasons,
        BTreeMap::from([
            (EligibilityReason::InterpreterContract, 2),
            (EligibilityReason::StatePreconditions, 2),
        ])
    );
    assert_eq!(
        report.case_features,
        BTreeMap::from([
            (CaseFeature::MappedMemory, 2),
            (CaseFeature::Reservation, 2),
        ])
    );
}

#[test]
fn assessed_refuses_a_saturated_eligibility_counter() {
    let mut report = report(0);
    report.undefined_cases = u64::MAX;
    let undefined = CaseAssessment::new(
        CaseEligibility::Undefined,
        EligibilityReason::ArchitecturallyUndefined,
        [],
    );

    assert_eq!(report.assessed(&undefined), overflow("case eligibility"));
}

#[test]
fn assessed_refuses_a_saturated_eligibility_distribution() {
    let mut report = report(0);
    report
        .distribution
        .eligibility
        .insert(CaseEligibility::Eligible, u64::MAX);

    assert_eq!(
        report.assessed(&eligible(EligibilityReason::InterpreterContract)),
        overflow("eligibility distribution")
    );
}

#[test]
fn assessed_refuses_a_saturated_reason_counter() {
    let mut report = report(0);
    report
        .eligibility_reasons
        .insert(EligibilityReason::DecoderRobustness, u64::MAX);

    assert_eq!(
        report.assessed(&eligible(EligibilityReason::DecoderRobustness)),
        overflow("eligibility reason")
    );
}

#[test]
fn assessed_refuses_a_saturated_feature_counter() {
    let mut report = report(0);
    report
        .case_features
        .insert(CaseFeature::ControlledFlow, u64::MAX);
    let assessment = CaseAssessment::new(
        CaseEligibility::Eligible,
        EligibilityReason::InterpreterContract,
        [CaseFeature::ControlledFlow],
    );

    assert_eq!(report.assessed(&assessment), overflow("case feature"));
}

#[test]
fn executed_sums_steps_tracks_the_deepest_case_and_counts_each_depth() {
    let mut report = report(0);

    assert_eq!(report.executed(5), Ok(()));
    assert_eq!(report.executed(3), Ok(()));
    assert_eq!(report.executed(0), Ok(()));
    assert_eq!(report.executed_steps, 8);
    assert_eq!(report.max_executed_depth, 5);
    assert_eq!(
        report.distribution.executed_depths,
        BTreeMap::from([(0, 1), (3, 1), (5, 1)])
    );
}

#[test]
fn executed_refuses_a_saturated_step_counter() {
    let mut report = report(0);
    report.executed_steps = u64::MAX;

    assert_eq!(report.executed(1), overflow("executed steps"));
}

#[test]
fn executed_refuses_a_saturated_depth_distribution() {
    let mut report = report(0);
    report.distribution.executed_depths.insert(4, u64::MAX);

    assert_eq!(report.executed(4), overflow("executed-depth distribution"));
}

#[test]
fn observe_case_retains_then_deduplicates_equal_observations() {
    let mut report = report(0);

    assert_eq!(
        report.observe_case(0, observation(1)),
        Ok(RetentionDecision::Retained)
    );
    assert_eq!(
        report.observe_case(9, observation(1)),
        Ok(RetentionDecision::Duplicate { representative: 0 })
    );
    assert_eq!(
        report.distribution.retention,
        BTreeMap::from([
            (RetentionClass::Retained, 1),
            (RetentionClass::Duplicate, 1)
        ])
    );
    assert_eq!(report.trial_metrics().retained, 1);
}

#[test]
fn observe_case_attaches_the_observation_to_findings_of_the_same_case() {
    let mut report = report(4);
    report
        .finding(finding(FindingKind::IllegalOutcome, 3))
        .unwrap();
    report
        .finding(finding(FindingKind::IllegalOutcome, 5))
        .unwrap();

    assert_eq!(
        report.observe_case(3, observation(2)),
        Ok(RetentionDecision::Retained)
    );
    assert_eq!(report.findings[0].observation, Some(observation(2)));
    assert_eq!(report.findings[1].observation, None);
}

#[test]
fn observe_case_refuses_a_saturated_retention_distribution() {
    let mut report = report(0);
    report
        .distribution
        .retention
        .insert(RetentionClass::Retained, u64::MAX);

    assert_eq!(
        report.observe_case(0, observation(1)),
        Err(InvariantError::CounterOverflow {
            counter: "retention distribution",
        })
    );
}

#[test]
fn observed_effects_count_each_occurrence_by_class() {
    let mut report = report(0);

    assert_eq!(
        report.observed_effects([
            EffectKind::MailboxSend,
            EffectKind::SharedWriteIntent,
            EffectKind::MailboxSend,
        ]),
        Ok(())
    );
    assert_eq!(report.observed_effects([]), Ok(()));
    assert_eq!(
        report.effect_classes,
        BTreeMap::from([
            (EffectKind::SharedWriteIntent, 1),
            (EffectKind::MailboxSend, 2),
        ])
    );
}

#[test]
fn observed_effects_refuse_a_saturated_class_counter() {
    let mut report = report(0);
    report
        .effect_classes
        .insert(EffectKind::DmaEnqueue, u64::MAX);

    assert_eq!(
        report.observed_effects([EffectKind::DmaEnqueue]),
        overflow("effect class")
    );
}

#[test]
fn metamorphic_relations_count_executions_and_skips_separately() {
    let mut report = report(0);

    assert_eq!(
        report.metamorphic_executed(CheckIdentity::PpuRecordCr0),
        Ok(())
    );
    assert_eq!(
        report.metamorphic_executed(CheckIdentity::PpuRecordCr0),
        Ok(())
    );
    assert_eq!(
        report.metamorphic_skipped(CheckIdentity::PpuRecordCr0),
        Ok(())
    );
    assert_eq!(
        report.metamorphic_executions,
        BTreeMap::from([(CheckIdentity::PpuRecordCr0, 2)])
    );
    assert_eq!(
        report.metamorphic_inapplicable,
        BTreeMap::from([(CheckIdentity::PpuRecordCr0, 1)])
    );
}

#[test]
fn metamorphic_counters_refuse_saturation() {
    let mut report = report(0);
    report
        .metamorphic_executions
        .insert(CheckIdentity::SpuNopFalseTarget, u64::MAX);
    report
        .metamorphic_inapplicable
        .insert(CheckIdentity::SpuNopFalseTarget, u64::MAX);

    assert_eq!(
        report.metamorphic_executed(CheckIdentity::SpuNopFalseTarget),
        overflow("metamorphic executions")
    );
    assert_eq!(
        report.metamorphic_skipped(CheckIdentity::SpuNopFalseTarget),
        overflow("inapplicable metamorphic relations")
    );
}

#[test]
fn eligibility_rate_is_none_when_the_classified_total_overflows() {
    let mut report = report(0);
    report.eligible_cases = u64::MAX;
    report.undefined_cases = 1;

    assert_eq!(report.eligibility_rate(), None);
}

#[test]
fn eligibility_rate_divides_by_every_classified_case() {
    let mut report = report(0);
    report.eligible_cases = 0;
    report.unsupported_cases = 3;
    report.undefined_cases = 2;

    assert_eq!(report.eligibility_rate(), Some((0, 5)));
}

#[test]
fn trial_metrics_read_the_attempted_distribution_not_the_case_counter() {
    let mut report = report(0);
    report.cases = 9;
    report.distribution.attempted = 3;
    report.eligible_cases = 2;
    report.executed_steps = 7;

    assert_eq!(
        report.trial_metrics(),
        TrialMetrics {
            seed: 7,
            attempted: 3,
            eligible: 2,
            executed: 7,
            retained: 0,
        }
    );
}

#[test]
fn findings_are_counted_beyond_the_retained_cap() {
    let mut report = report(2);

    for case_index in 0..3 {
        report
            .finding(finding(FindingKind::IllegalEffect, case_index))
            .unwrap();
    }

    assert_eq!(
        report.finding_counts,
        BTreeMap::from([(FindingKind::IllegalEffect, 3)])
    );
    assert_eq!(
        report
            .findings
            .iter()
            .map(|finding| finding.replay.case_index)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert!(!report.is_clean());
}

#[test]
fn a_zero_cap_retains_no_finding_but_still_counts_it() {
    let mut report = report(0);

    report
        .finding(finding(FindingKind::Nondeterministic, 4))
        .unwrap();

    assert!(report.findings.is_empty());
    assert_eq!(
        report.finding_counts,
        BTreeMap::from([(FindingKind::Nondeterministic, 1)])
    );
}

#[test]
fn finding_refuses_a_saturated_kind_counter_without_retaining_it() {
    let mut report = report(4);
    report
        .finding_counts
        .insert(FindingKind::TargetPanic, u64::MAX);

    assert_eq!(
        report.finding(finding(FindingKind::TargetPanic, 0)),
        overflow("finding count")
    );
    assert!(report.findings.is_empty());
}

#[test]
fn an_inapplicability_finding_makes_the_report_unclean() {
    let mut report = report(0);
    report
        .finding(finding(FindingKind::Unsupported, 0))
        .unwrap();

    assert!(!report.is_clean());
}

#[test]
fn an_empty_report_completes_clean() {
    assert_eq!(outcome_for(&[], 0, 0, 0), RunOutcome::CleanCompletion);
}

#[test]
fn eligible_cases_without_findings_complete_clean_even_beside_inapplicable_cases() {
    assert_eq!(outcome_for(&[], 1, 1, 1), RunOutcome::CleanCompletion);
}

#[test]
fn a_target_panic_outranks_every_other_finding() {
    assert_eq!(
        outcome_for(
            &[
                FindingKind::IllegalOutcome,
                FindingKind::TargetPanic,
                FindingKind::Unsupported,
            ],
            1,
            0,
            0
        ),
        RunOutcome::TargetPanic
    );
}

#[test]
fn every_semantic_finding_kind_is_a_semantic_finding() {
    for kind in [
        FindingKind::Nondeterministic,
        FindingKind::MetamorphicViolation,
        FindingKind::IllegalOutcome,
        FindingKind::IllegalEffect,
        FindingKind::IllegalFootprint,
        FindingKind::InvalidProgramCounter,
    ] {
        assert_eq!(
            outcome_for(&[kind, FindingKind::Unsupported], 1, 1, 1),
            RunOutcome::SemanticFinding,
            "{kind:?}"
        );
    }
}

#[test]
fn inapplicability_findings_classify_by_which_kinds_appear() {
    assert_eq!(
        outcome_for(&[FindingKind::Unsupported], 0, 0, 0),
        RunOutcome::UnsupportedCase
    );
    assert_eq!(
        outcome_for(&[FindingKind::Undefined], 0, 0, 0),
        RunOutcome::UndefinedCase
    );
    assert_eq!(
        outcome_for(&[FindingKind::Unsupported, FindingKind::Undefined], 0, 0, 0),
        RunOutcome::UnsupportedAndUndefinedCases
    );
}

#[test]
fn inapplicability_counters_classify_without_findings() {
    assert_eq!(outcome_for(&[], 0, 2, 0), RunOutcome::UnsupportedCase);
    assert_eq!(outcome_for(&[], 0, 0, 2), RunOutcome::UndefinedCase);
    assert_eq!(
        outcome_for(&[], 0, 1, 1),
        RunOutcome::UnsupportedAndUndefinedCases
    );
}

#[test]
fn inapplicability_findings_beside_eligible_cases_classify_by_which_kinds_appear() {
    assert_eq!(
        outcome_for(&[FindingKind::Unsupported], 1, 0, 0),
        RunOutcome::UnsupportedCase
    );
    assert_eq!(
        outcome_for(&[FindingKind::Undefined], 1, 1, 0),
        RunOutcome::UnsupportedAndUndefinedCases
    );
}

#[test]
fn completed_runs_keep_the_report() {
    let mut report = report(0);
    report.cases = 5;

    assert_eq!(FuzzRun::completed(report.clone()).report, report);
}

#[test]
fn failed_runs_wrap_the_error_and_keep_the_partial_report() {
    let mut report = report(0);
    report.cases = 5;
    let run = FuzzRun::failed(report.clone(), ConfigurationError::ZeroIterations);

    assert_eq!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(ConfigurationError::ZeroIterations))
    );
    assert_eq!(run.report, report);
}

#[test]
fn cancelled_runs_keep_the_partial_report() {
    let mut report = report(0);
    report.cases = 5;
    let run = FuzzRun::cancelled(report.clone());

    assert_eq!(run.outcome, RunOutcome::Cancelled);
    assert_eq!(run.report, report);
}

#[test]
fn fuzz_targets_serialize_by_variant_name() {
    for (target, name) in [
        (FuzzTarget::PpuInstruction, "PpuInstruction"),
        (FuzzTarget::PpuSequence, "PpuSequence"),
        (FuzzTarget::SpuInstruction, "SpuInstruction"),
        (FuzzTarget::SpuSequence, "SpuSequence"),
    ] {
        assert_eq!(serde_json::to_value(target).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<FuzzTarget>(json!(name)).unwrap(),
            target
        );
    }
    assert_eq!(
        serde_json::from_value::<FuzzTarget>(json!("ppu_sequence"))
            .unwrap_err()
            .to_string(),
        concat!(
            "unknown variant `ppu_sequence`, expected one of `PpuInstruction`, ",
            "`PpuSequence`, `SpuInstruction`, `SpuSequence`"
        )
    );
}

#[test]
fn fuzz_targets_order_by_declaration() {
    let declared = [
        FuzzTarget::PpuInstruction,
        FuzzTarget::PpuSequence,
        FuzzTarget::SpuInstruction,
        FuzzTarget::SpuSequence,
    ];
    let mut reversed = declared;
    reversed.reverse();
    reversed.sort();

    assert_eq!(reversed, declared);
}

#[test]
fn finding_kinds_order_by_declaration() {
    let declared = [
        FindingKind::TargetPanic,
        FindingKind::Nondeterministic,
        FindingKind::MetamorphicViolation,
        FindingKind::IllegalOutcome,
        FindingKind::IllegalEffect,
        FindingKind::IllegalFootprint,
        FindingKind::InvalidProgramCounter,
        FindingKind::Unsupported,
        FindingKind::Undefined,
    ];
    let mut reversed = declared;
    reversed.reverse();
    reversed.sort();

    assert_eq!(reversed, declared);
}

#[test]
fn divergence_classes_order_by_declaration() {
    let declared = [
        DivergenceClass::TargetPanic,
        DivergenceClass::ArchitecturalState,
        DivergenceClass::ReferenceDisagreement,
        DivergenceClass::Outcome,
        DivergenceClass::Effect,
        DivergenceClass::ControlFlow,
        DivergenceClass::Unsupported,
        DivergenceClass::Undefined,
    ];
    let mut reversed = declared;
    reversed.reverse();
    reversed.sort();

    assert_eq!(reversed, declared);
}

#[test]
fn check_and_outcome_identities_span_their_declared_ranges() {
    assert!(CheckIdentity::PpuDecoder < CheckIdentity::SpuDecoder);
    assert!(CheckIdentity::LegalOutcome < CheckIdentity::LegalEffect);
    assert!(CheckIdentity::ProgramCounter < CheckIdentity::ExternalReference);
    assert!(OutcomeIdentity::PpuContinue < OutcomeIdentity::PpuCommitRefusal);
    assert!(OutcomeIdentity::PpuCommitRefusal < OutcomeIdentity::SpuContinue);
    assert!(OutcomeIdentity::SpuFault < OutcomeIdentity::SpuDecodeRefusal);
}

#[test]
fn ppu_identities_order_before_spu_identities() {
    assert!(PPU_ORI < SPU_NOP);
}

#[test]
fn fingerprints_order_by_target_before_check() {
    let earlier_target = SemanticFingerprint {
        target: FuzzTarget::PpuSequence,
        instruction_kind: None,
        check: CheckIdentity::ExternalReference,
        divergence: DivergenceClass::Undefined,
        outcome: None,
        effect: None,
    };
    let later_target = SemanticFingerprint {
        target: FuzzTarget::SpuInstruction,
        instruction_kind: None,
        check: CheckIdentity::PpuDecoder,
        divergence: DivergenceClass::TargetPanic,
        outcome: None,
        effect: None,
    };

    assert!(earlier_target < later_target);
}
