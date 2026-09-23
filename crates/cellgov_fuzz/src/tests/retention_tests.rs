use std::collections::BTreeSet;

use cellgov_effects::EffectKind;
use cellgov_spu::instruction::SpuInstructionKind;

use super::*;

fn observation(
    kind: SpuInstructionKind,
    depth: u64,
    transition: StateTransitionClass,
    asymmetry: CrossReferenceAsymmetry,
) -> SemanticObservation {
    SemanticObservation {
        first_instruction_kind: Some(InstructionIdentity::Spu(kind)),
        instruction_kinds: BTreeSet::from([InstructionIdentity::Spu(kind)]),
        operands: OperandAliasClass::Ordinary,
        eligibility: CaseEligibility::Eligible,
        outcome: Some(OutcomeIdentity::SpuContinue),
        state_transition: transition,
        effects: BTreeSet::new(),
        boundaries: BTreeSet::new(),
        sequence_depth: depth,
        asymmetry,
    }
}

fn retained_score(retention: &RetainedCases, case_index: u64) -> u64 {
    retention
        .entries()
        .find(|entry| entry.case_index == case_index)
        .unwrap()
        .score
}

#[test]
fn retention_defaults_are_stable_and_validated() {
    let defaults = RetentionConfig::default();

    assert_eq!(
        defaults,
        RetentionConfig {
            capacity: 256,
            per_kind_capacity: 8,
            novelty_weight: 8,
            rarity_weight: 4,
            asymmetry_weight: 16,
            policy: ExplorationPolicy::Balanced,
        }
    );
    assert_eq!(defaults.validate(), Ok(()));
    assert_eq!(
        RetentionConfig {
            capacity: 0,
            ..defaults
        }
        .validate(),
        Err(RetentionConfigError::ZeroCapacity)
    );
    assert_eq!(
        RetentionConfig {
            per_kind_capacity: defaults.capacity + 1,
            ..defaults
        }
        .validate(),
        Err(RetentionConfigError::InvalidPerKindCapacity {
            per_kind: defaults.capacity + 1,
            total: defaults.capacity,
        })
    );
    assert_eq!(
        RetentionConfig {
            per_kind_capacity: 0,
            ..defaults
        }
        .validate(),
        Err(RetentionConfigError::InvalidPerKindCapacity {
            per_kind: 0,
            total: defaults.capacity,
        })
    );
    assert_eq!(
        RetentionConfig {
            novelty_weight: 0,
            rarity_weight: 0,
            asymmetry_weight: 0,
            ..defaults
        }
        .validate(),
        Err(RetentionConfigError::ZeroWeights)
    );
}

#[test]
fn equal_observations_share_one_representative() {
    let mut retention = RetainedCases::new(RetentionConfig::default()).unwrap();
    let observed = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );

    assert_eq!(
        retention.consider(7, observed.clone()),
        RetentionDecision::Retained
    );
    assert_eq!(
        retention.consider(9, observed),
        RetentionDecision::Duplicate { representative: 7 }
    );
    assert_eq!(retention.entries().len(), 1);
    assert_eq!(retention.entries().next().unwrap().occurrences, 2);
}

#[test]
fn terminal_decode_refusals_do_not_deduplicate_as_completion() {
    let mut retention = RetainedCases::new(RetentionConfig::default()).unwrap();
    let completed = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );
    let refused = SemanticObservation {
        outcome: Some(OutcomeIdentity::SpuDecodeRefusal),
        ..completed.clone()
    };

    assert_eq!(
        retention.consider(0, completed),
        RetentionDecision::Retained
    );
    assert_eq!(retention.consider(1, refused), RetentionDecision::Retained);
    assert_eq!(retention.entries().len(), 2);
}

#[test]
fn case_indices_cannot_silently_replace_an_unrelated_observation() {
    let mut retention = RetainedCases::new(RetentionConfig::default()).unwrap();
    let first = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );
    let second = observation(
        SpuInstructionKind::Lnop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );

    assert_eq!(
        retention.consider(7, first.clone()),
        RetentionDecision::Retained
    );
    assert_eq!(retention.consider(7, second), RetentionDecision::Rejected);
    assert_eq!(retention.entries().next().unwrap().observation, first);
}

#[test]
fn equal_inputs_produce_equal_retention_and_schedule() {
    let config = RetentionConfig {
        capacity: 3,
        per_kind_capacity: 2,
        ..RetentionConfig::default()
    };
    let inputs = [
        observation(
            SpuInstructionKind::Nop,
            1,
            StateTransitionClass::Unchanged,
            CrossReferenceAsymmetry::None,
        ),
        observation(
            SpuInstructionKind::Nop,
            2,
            StateTransitionClass::ArchitecturalState,
            CrossReferenceAsymmetry::None,
        ),
        observation(
            SpuInstructionKind::Lnop,
            1,
            StateTransitionClass::Unchanged,
            CrossReferenceAsymmetry::State,
        ),
        observation(
            SpuInstructionKind::Sync,
            3,
            StateTransitionClass::Effect,
            CrossReferenceAsymmetry::Effect,
        ),
    ];
    let run = || {
        let mut retention = RetainedCases::new(config).unwrap();
        let decisions = inputs
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, observed)| retention.consider(index as u64, observed))
            .collect::<Vec<_>>();
        (decisions, retention.scheduled_cases(), retention)
    };

    assert_eq!(run(), run());
    assert_eq!(
        {
            let (decisions, schedule, _) = run();
            (decisions, schedule)
        },
        (
            vec![
                RetentionDecision::Retained,
                RetentionDecision::Retained,
                RetentionDecision::Retained,
                RetentionDecision::Replaced { evicted: 1 },
            ],
            vec![0, 3, 2],
        )
    );
}

#[test]
fn exact_semantic_axis_sets_receive_novelty() {
    let config = RetentionConfig {
        capacity: 9,
        per_kind_capacity: 9,
        novelty_weight: 1,
        rarity_weight: 0,
        asymmetry_weight: 0,
        policy: ExplorationPolicy::Balanced,
    };

    let mut kinds = RetainedCases::new(config).unwrap();
    let mut nop = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );
    let lnop = observation(
        SpuInstructionKind::Lnop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );
    kinds.consider(0, nop.clone());
    kinds.consider(1, lnop.clone());
    nop.instruction_kinds
        .insert(InstructionIdentity::Spu(SpuInstructionKind::Lnop));
    kinds.consider(2, nop);
    assert_eq!(retained_score(&kinds, 2), 1);

    let mut effects = RetainedCases::new(config).unwrap();
    let mut clock = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::Effect,
        CrossReferenceAsymmetry::None,
    );
    clock.effects.insert(EffectKind::ClockRead);
    let mut fault = clock.clone();
    fault.effects = BTreeSet::from([EffectKind::FaultRaised]);
    effects.consider(0, clock.clone());
    effects.consider(1, fault.clone());
    clock.effects.extend(fault.effects);
    effects.consider(2, clock);
    assert_eq!(retained_score(&effects, 2), 1);

    let mut boundaries = RetainedCases::new(config).unwrap();
    let mut operand = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );
    operand.boundaries.insert(BoundaryClass::Operand);
    let mut memory = operand.clone();
    memory.boundaries = BTreeSet::from([BoundaryClass::Memory]);
    boundaries.consider(0, operand.clone());
    boundaries.consider(1, memory.clone());
    operand.boundaries.extend(memory.boundaries);
    boundaries.consider(2, operand);
    assert_eq!(retained_score(&boundaries, 2), 1);
}

#[test]
fn rejected_and_evicted_axes_can_be_novel_again() {
    let config = RetentionConfig {
        capacity: 1,
        per_kind_capacity: 1,
        novelty_weight: 1,
        rarity_weight: 0,
        asymmetry_weight: 0,
        policy: ExplorationPolicy::Balanced,
    };
    let mut retention = RetainedCases::new(config).unwrap();
    let mut baseline = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );
    baseline.effects.insert(EffectKind::ClockRead);
    assert_eq!(
        retention.consider(0, baseline.clone()),
        RetentionDecision::Retained
    );

    let mut rejected = baseline.clone();
    rejected.state_transition = StateTransitionClass::Effect;
    rejected.effects.insert(EffectKind::FaultRaised);
    assert_eq!(retention.consider(1, rejected), RetentionDecision::Rejected);

    let mut replacement = observation(
        SpuInstructionKind::Lnop,
        1,
        StateTransitionClass::Effect,
        CrossReferenceAsymmetry::None,
    );
    replacement.effects.insert(EffectKind::FaultRaised);
    assert!(matches!(
        retention.consider(2, replacement),
        RetentionDecision::Replaced { evicted: 0 }
    ));
    assert_eq!(retained_score(&retention, 2), 3);

    assert!(matches!(
        retention.consider(3, baseline),
        RetentionDecision::Replaced { evicted: 2 }
    ));
    assert_eq!(retained_score(&retention, 3), 3);
}

#[test]
fn quotas_preserve_rare_kinds_from_dense_classes() {
    let config = RetentionConfig {
        capacity: 2,
        per_kind_capacity: 2,
        novelty_weight: 8,
        rarity_weight: 8,
        asymmetry_weight: 32,
        policy: ExplorationPolicy::Balanced,
    };
    let mut retention = RetainedCases::new(config).unwrap();
    for (case_index, depth, transition) in [
        (0, 1, StateTransitionClass::Unchanged),
        (1, 2, StateTransitionClass::ArchitecturalState),
    ] {
        retention.consider(
            case_index,
            observation(
                SpuInstructionKind::Nop,
                depth,
                transition,
                CrossReferenceAsymmetry::None,
            ),
        );
    }
    assert!(matches!(
        retention.consider(
            2,
            observation(
                SpuInstructionKind::Lnop,
                1,
                StateTransitionClass::Unchanged,
                CrossReferenceAsymmetry::None,
            ),
        ),
        RetentionDecision::Replaced { .. }
    ));

    let retained = retention.entries().collect::<Vec<_>>();
    assert!(retained.iter().any(|entry| {
        entry
            .observation
            .instruction_kinds
            .contains(&InstructionIdentity::Spu(SpuInstructionKind::Lnop))
    }));
    assert!(
        retained
            .iter()
            .filter(|entry| entry
                .observation
                .instruction_kinds
                .contains(&InstructionIdentity::Spu(SpuInstructionKind::Nop)))
            .count()
            == 1
    );
}

#[test]
fn per_kind_quota_uses_the_first_executed_kind() {
    let config = RetentionConfig {
        capacity: 2,
        per_kind_capacity: 1,
        ..RetentionConfig::default()
    };
    let nop = InstructionIdentity::Spu(SpuInstructionKind::Nop);
    let lnop = InstructionIdentity::Spu(SpuInstructionKind::Lnop);
    let (ordered_first, other) = if nop < lnop { (nop, lnop) } else { (lnop, nop) };
    let mut sequence = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );
    sequence.first_instruction_kind = Some(other);
    sequence.instruction_kinds = BTreeSet::from([ordered_first, other]);
    let mut candidate = sequence.clone();
    candidate.first_instruction_kind = Some(ordered_first);
    candidate.instruction_kinds = BTreeSet::from([ordered_first]);
    candidate.sequence_depth = 2;

    let mut retention = RetainedCases::new(config).unwrap();
    assert_eq!(retention.consider(0, sequence), RetentionDecision::Retained);
    assert_eq!(
        retention.consider(1, candidate),
        RetentionDecision::Retained
    );
}

#[test]
fn asymmetry_policy_schedules_disagreement_first() {
    let config = RetentionConfig {
        capacity: 4,
        per_kind_capacity: 4,
        novelty_weight: 1,
        rarity_weight: 0,
        asymmetry_weight: 1,
        policy: ExplorationPolicy::Balanced,
    };
    let inputs = [
        observation(
            SpuInstructionKind::Nop,
            1,
            StateTransitionClass::Unchanged,
            CrossReferenceAsymmetry::None,
        ),
        observation(
            SpuInstructionKind::Nop,
            2,
            StateTransitionClass::ArchitecturalState,
            CrossReferenceAsymmetry::State,
        ),
    ];
    let schedule = |policy| {
        let mut retention = RetainedCases::new(RetentionConfig { policy, ..config }).unwrap();
        for (case_index, observed) in inputs.iter().cloned().enumerate() {
            retention.consider(case_index as u64, observed);
        }
        retention.scheduled_cases()
    };

    assert_eq!(schedule(ExplorationPolicy::Balanced).first(), Some(&0));
    assert_eq!(
        schedule(ExplorationPolicy::AsymmetryFirst).first(),
        Some(&1)
    );
}

#[test]
fn asymmetry_precedence_survives_rare_kind_replacement() {
    let config = RetentionConfig {
        capacity: 1,
        per_kind_capacity: 1,
        novelty_weight: 1,
        rarity_weight: 1,
        asymmetry_weight: 1,
        policy: ExplorationPolicy::AsymmetryFirst,
    };
    let mut retention = RetainedCases::new(config).unwrap();
    retention.consider(
        0,
        observation(
            SpuInstructionKind::Nop,
            1,
            StateTransitionClass::Unchanged,
            CrossReferenceAsymmetry::State,
        ),
    );

    assert_eq!(
        retention.consider(
            1,
            observation(
                SpuInstructionKind::Lnop,
                1,
                StateTransitionClass::Unchanged,
                CrossReferenceAsymmetry::None,
            ),
        ),
        RetentionDecision::Rejected
    );
    assert_eq!(retention.scheduled_cases(), [0]);
}

#[test]
fn novelty_policy_uses_lexicographic_precedence() {
    let config = RetentionConfig {
        capacity: 3,
        per_kind_capacity: 3,
        novelty_weight: 1,
        rarity_weight: 0,
        asymmetry_weight: 1,
        policy: ExplorationPolicy::Balanced,
    };
    let baseline = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );
    let asymmetric = SemanticObservation {
        asymmetry: CrossReferenceAsymmetry::State,
        ..baseline.clone()
    };
    let more_novel = SemanticObservation {
        operands: OperandAliasClass::Boundary,
        state_transition: StateTransitionClass::ArchitecturalState,
        ..baseline.clone()
    };
    let schedule = |policy| {
        let mut retention = RetainedCases::new(RetentionConfig { policy, ..config }).unwrap();
        for (case_index, observed) in [baseline.clone(), asymmetric.clone(), more_novel.clone()]
            .into_iter()
            .enumerate()
        {
            retention.consider(case_index as u64, observed);
        }
        retention.scheduled_cases()
    };

    assert_eq!(schedule(ExplorationPolicy::Balanced), [0, 1, 2]);
    assert_eq!(schedule(ExplorationPolicy::NoveltyFirst), [0, 2, 1]);
}

#[test]
fn zero_weight_disables_policy_precedence() {
    let config = RetentionConfig {
        capacity: 2,
        per_kind_capacity: 2,
        novelty_weight: 1,
        rarity_weight: 0,
        asymmetry_weight: 0,
        policy: ExplorationPolicy::AsymmetryFirst,
    };
    let mut retention = RetainedCases::new(config).unwrap();
    retention.consider(
        0,
        observation(
            SpuInstructionKind::Nop,
            1,
            StateTransitionClass::Unchanged,
            CrossReferenceAsymmetry::None,
        ),
    );
    retention.consider(
        1,
        observation(
            SpuInstructionKind::Nop,
            2,
            StateTransitionClass::ArchitecturalState,
            CrossReferenceAsymmetry::State,
        ),
    );

    assert_eq!(retention.scheduled_cases().first(), Some(&0));
}

#[test]
fn execution_depth_prevents_kind_coverage_from_being_vacuous() {
    let mut retention = RetainedCases::new(RetentionConfig::default()).unwrap();
    let mut apparent = observation(
        SpuInstructionKind::Nop,
        0,
        StateTransitionClass::Unchanged,
        CrossReferenceAsymmetry::None,
    );
    apparent.first_instruction_kind = None;
    apparent.instruction_kinds.clear();
    let executed = observation(
        SpuInstructionKind::Nop,
        1,
        StateTransitionClass::ArchitecturalState,
        CrossReferenceAsymmetry::None,
    );

    retention.consider(0, apparent);
    retention.consider(1, executed);

    assert_eq!(retention.entries().len(), 2);
    assert!(retention
        .entries()
        .find(|entry| entry.observation.sequence_depth == 0)
        .unwrap()
        .observation
        .instruction_kinds
        .is_empty());
    assert_eq!(
        retention
            .entries()
            .map(|entry| entry.observation.sequence_depth)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([0, 1])
    );
}

#[test]
fn kind_reach_does_not_count_as_execution() {
    let mut report = FuzzReport::new(
        FuzzTarget::SpuInstruction,
        7,
        GenerationStrategy::Structured,
        RetentionConfig::default(),
        1,
        1,
    );
    report.considered().unwrap();
    report
        .reached(InstructionIdentity::Spu(SpuInstructionKind::Nop))
        .unwrap();

    assert_eq!(report.instruction_kinds.len(), 1);
    assert!(report.distribution.eligibility.is_empty());
    assert!(report.distribution.executed_depths.is_empty());
    assert_eq!(report.retained_cases.entries().len(), 0);
}

#[test]
fn report_keeps_attempt_eligibility_and_execution_distributions_separate() {
    let mut report = FuzzReport::new(
        FuzzTarget::SpuInstruction,
        7,
        GenerationStrategy::Structured,
        RetentionConfig::default(),
        1,
        1,
    );
    report.considered().unwrap();
    report.considered().unwrap();
    report
        .assessed(&CaseAssessment::new(
            CaseEligibility::Eligible,
            EligibilityReason::InterpreterContract,
            [CaseFeature::OperandBoundary],
        ))
        .unwrap();
    report.executed(3).unwrap();

    assert_eq!(report.distribution.attempted, 2);
    assert_eq!(
        report
            .distribution
            .eligibility
            .get(&CaseEligibility::Eligible),
        Some(&1)
    );
    assert_eq!(report.distribution.executed_depths.get(&3), Some(&1));
}

#[test]
fn every_engine_records_executed_semantics_in_retained_cases() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 16,
            },
            ..CampaignSchedule::default()
        },
        sequence_words: 4,
        ..FuzzConfig::default()
    };
    let runs = [
        ppu::run_instructions(config),
        ppu::run_sequences(config),
        spu::run_instructions(config),
        spu::run_sequences(config),
    ];

    for run in runs {
        let assessed =
            run.report.eligible_cases + run.report.unsupported_cases + run.report.undefined_cases;
        let executed_cases = run
            .report
            .distribution
            .executed_depths
            .values()
            .copied()
            .sum::<u64>();
        let retention_decisions = run
            .report
            .distribution
            .retention
            .values()
            .copied()
            .sum::<u64>();

        assert_eq!(run.report.distribution.attempted, 16);
        assert_eq!(assessed, 16);
        assert_eq!(executed_cases, assessed);
        assert_eq!(retention_decisions, assessed);
        assert!(run.report.retained_cases.entries().len() > 0);
    }
}

#[test]
fn equal_campaign_inputs_produce_equal_engine_retained_cases_and_schedules() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 41,
                count: 12,
            },
            ..CampaignSchedule::default()
        },
        sequence_words: 4,
        ..FuzzConfig::default()
    };
    let runners: [fn(FuzzConfig) -> FuzzRun; 4] = [
        ppu::run_instructions,
        ppu::run_sequences,
        spu::run_instructions,
        spu::run_sequences,
    ];

    for runner in runners {
        let first = runner(config);
        let second = runner(config);

        assert_eq!(first.report.retained_cases, second.report.retained_cases);
        assert_eq!(
            first.report.retained_cases.scheduled_cases(),
            second.report.retained_cases.scheduled_cases()
        );
        assert_eq!(first.report.distribution, second.report.distribution);
    }
}

#[test]
fn repeated_trials_require_equal_budgets_and_report_distributions() {
    let trials = [
        TrialMetrics {
            seed: 9,
            attempted: 100,
            eligible: 61,
            executed: 80,
            retained: 17,
        },
        TrialMetrics {
            seed: 3,
            attempted: 100,
            eligible: 54,
            executed: 72,
            retained: 20,
        },
        TrialMetrics {
            seed: 7,
            attempted: 100,
            eligible: 59,
            executed: 76,
            retained: 18,
        },
    ];

    assert_eq!(
        EvaluationDistribution::from_trials(trials).unwrap(),
        EvaluationDistribution {
            attempted_budget: 100,
            seeds: vec![3, 7, 9],
            eligible: vec![54, 59, 61],
            executed: vec![72, 76, 80],
            retained: vec![17, 18, 20],
        }
    );
    assert!(matches!(
        EvaluationDistribution::from_trials([trials[0]]),
        Err(EvaluationError::TooFewTrials { found: 1 })
    ));
    assert_eq!(
        EvaluationDistribution::from_trials([trials[0], trials[0]]),
        Err(EvaluationError::RepeatedSeed)
    );
    assert_eq!(
        EvaluationDistribution::from_trials([
            trials[0],
            TrialMetrics {
                seed: 10,
                attempted: 99,
                ..trials[1]
            }
        ]),
        Err(EvaluationError::UnequalBudget {
            expected: 100,
            found: 99,
            seed: 10,
        })
    );
}
