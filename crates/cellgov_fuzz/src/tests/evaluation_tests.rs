use super::*;

use std::cmp::Ordering;

use crate::reduce::{ReductionPolicy, ReductionRequest};
use crate::report::{FuzzReport, FuzzRun, RunOutcome};
use crate::seeded::{seed, SeededDefect};
use crate::{
    CampaignSchedule, CampaignShard, CaseRange, ConfigurationError, FuzzError, FuzzTarget,
    GenerationStrategy, InvariantError, RetentionConfig,
};

const CASES: u64 = 48;
const FIRST_SEED: u64 = 100;
const TRIALS: u32 = 6;

fn plan(target: FuzzTarget, strategy: GenerationStrategy) -> EvaluationPlan {
    EvaluationPlan {
        sequence_words: 6,
        ..EvaluationPlan::new(target, strategy, CASES, FIRST_SEED, TRIALS)
    }
}

fn environment() -> EvaluationEnvironment {
    EvaluationEnvironment {
        crate_version: "test".into(),
        os: "test".into(),
        arch: "test".into(),
        workers: 1,
    }
}

fn results(plan: EvaluationPlan) -> EvaluationResults {
    let trials = plan
        .seeds
        .iter()
        .map(|seed| run_trial(&plan, *seed).expect("valid plan"))
        .collect();
    EvaluationResults::from_trials(plan, environment(), trials).expect("complete results")
}

fn distribution(results: &EvaluationResults, metric: Metric) -> &Distribution {
    results
        .summary
        .distributions
        .get(&metric)
        .unwrap_or_else(|| panic!("{metric:?} was not sampled"))
}

#[test]
fn a_plan_lists_consecutive_seeds_and_runs_each_at_the_same_budget() {
    let plan = plan(FuzzTarget::PpuInstruction, GenerationStrategy::Structured);
    assert_eq!(plan.seeds, [100, 101, 102, 103, 104, 105]);
    assert_eq!(plan.validate(), Ok(()));
    let config = plan.trial_config(103);
    assert_eq!(config.seed, 103);
    assert_eq!(
        config.schedule,
        CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: CASES
            },
            shard: CampaignShard::ALL,
            cancellation: None,
        }
    );
    assert_eq!(config.sequence_words, 6);
}

#[test]
fn plans_refuse_too_few_trials_repeated_seeds_empty_budgets_and_refused_campaigns() {
    let base = plan(FuzzTarget::PpuSequence, GenerationStrategy::Structured);
    let one = EvaluationPlan {
        seeds: vec![100],
        ..base.clone()
    };
    assert_eq!(
        one.validate(),
        Err(EvaluationPlanError::TooFewTrials { found: 1 })
    );
    let repeated = EvaluationPlan {
        seeds: vec![100, 101, 100],
        ..base.clone()
    };
    assert_eq!(
        repeated.validate(),
        Err(EvaluationPlanError::RepeatedSeed { seed: 100 })
    );
    let empty = EvaluationPlan {
        budget: TrialBudget { cases: 0 },
        ..base.clone()
    };
    assert_eq!(empty.validate(), Err(EvaluationPlanError::EmptyBudget));
    let refused = EvaluationPlan {
        sequence_words: 0,
        ..base
    };
    assert_eq!(
        refused.validate(),
        Err(EvaluationPlanError::Configuration(
            ConfigurationError::ZeroSequenceWords
        ))
    );
    assert_eq!(
        run_trial(&refused, 100).unwrap_err(),
        EvaluationPlanError::Configuration(ConfigurationError::ZeroSequenceWords)
    );
}

#[test]
fn a_trial_replays_from_the_plan_and_its_seed_alone() {
    let plan = plan(FuzzTarget::SpuInstruction, GenerationStrategy::Structured);
    let first = run_trial(&plan, 100).expect("runs");
    assert_eq!(run_trial(&plan, 100).expect("runs"), first);
    assert_eq!(first.outcome, TrialOutcome::Clean);
    assert_eq!(first.counts.attempted, CASES);
    assert!(first.counts.eligible > 0);
    assert!(first.counts.executed_steps >= first.counts.eligible);
    assert!(first.counts.instruction_kinds > 0);
    assert!(first.findings.is_empty());
    assert_eq!(first.unique_fingerprints, 0);
    assert_eq!(first.first_finding_offset, None);
    assert_eq!(first.reduction, None);
    assert_eq!(first.wall_ms, None);
    let second = run_trial(&plan, 101).expect("runs");
    assert_ne!(
        second.counts, first.counts,
        "two seeds ran the same campaign"
    );
}

#[test]
fn trial_outcomes_map_every_run_outcome() {
    let plan = plan(FuzzTarget::PpuInstruction, GenerationStrategy::Structured);
    let report = || {
        FuzzReport::new(
            FuzzTarget::PpuInstruction,
            100,
            GenerationStrategy::Structured,
            RetentionConfig::default(),
            4,
            1,
        )
    };
    for (outcome, expected) in [
        (RunOutcome::CleanCompletion, TrialOutcome::Clean),
        (RunOutcome::SemanticFinding, TrialOutcome::SemanticFinding),
        (RunOutcome::TargetPanic, TrialOutcome::TargetPanic),
        (RunOutcome::Cancelled, TrialOutcome::Cancelled),
        (RunOutcome::NoEligibleCases, TrialOutcome::NoEligibleCases),
        (RunOutcome::UnsupportedCase, TrialOutcome::Inapplicable),
        (RunOutcome::UndefinedCase, TrialOutcome::Inapplicable),
        (
            RunOutcome::UnsupportedAndUndefinedCases,
            TrialOutcome::Inapplicable,
        ),
        (
            RunOutcome::HarnessFailure(FuzzError::from(InvariantError::EmptyGeneratedSequence)),
            TrialOutcome::HarnessFailure {
                message: FuzzError::from(InvariantError::EmptyGeneratedSequence).to_string(),
            },
        ),
    ] {
        let trial = run_trial_with(&plan, 100, |config| {
            assert_eq!(config.seed, 100);
            FuzzRun {
                outcome: outcome.clone(),
                report: report(),
            }
        });
        assert_eq!(trial.outcome, expected, "{outcome:?}");
    }
}

#[test]
fn results_hold_every_planned_trial_in_order_and_summarize_them() {
    let results = results(plan(
        FuzzTarget::PpuInstruction,
        GenerationStrategy::Structured,
    ));
    assert_eq!(results.schema_version, EVALUATION_SCHEMA_VERSION);
    assert_eq!(
        results
            .trials
            .iter()
            .map(|trial| trial.seed)
            .collect::<Vec<_>>(),
        results.plan.seeds
    );
    assert!(results
        .trials
        .iter()
        .all(|trial| trial.outcome == TrialOutcome::Clean));
    assert_eq!(results.summary.trials, u64::from(TRIALS));
    assert_eq!(results.summary.cases_per_trial, CASES);
    for metric in [
        Metric::Eligible,
        Metric::ExecutedSteps,
        Metric::MaxExecutedDepth,
        Metric::InstructionKinds,
        Metric::EffectClasses,
        Metric::MetamorphicExecutions,
        Metric::Retained,
        Metric::Findings,
        Metric::UniqueFingerprints,
        Metric::InvalidCases,
    ] {
        let distribution = distribution(&results, metric);
        assert_eq!(distribution.samples.len(), TRIALS as usize, "{metric:?}");
        let mut sorted = results.samples(metric);
        sorted.sort_unstable();
        assert_eq!(distribution.samples, sorted, "{metric:?}");
    }
    for absent in [
        Metric::FirstFindingOffset,
        Metric::ReductionEvaluations,
        Metric::WallMs,
    ] {
        assert!(
            !results.summary.distributions.contains_key(&absent),
            "{absent:?} was sampled by a clean untimed trial"
        );
    }
    let json = serde_json::to_string_pretty(&results).expect("serializes");
    assert_eq!(
        EvaluationResults::parse_json(&json).expect("parses"),
        results
    );
}

#[test]
fn results_refuse_dropped_added_reordered_and_off_budget_trials_and_edited_summaries() {
    let results = results(plan(
        FuzzTarget::SpuInstruction,
        GenerationStrategy::Structured,
    ));
    let rebuild = |trials: Vec<TrialRecord>| EvaluationResults {
        trials,
        ..results.clone()
    };

    let mut dropped = results.trials.clone();
    dropped.pop();
    assert!(matches!(
        rebuild(dropped).validate(),
        Err(ResultsError::MissingTrial { seed: 105 })
    ));

    let mut added = results.trials.clone();
    added.push(TrialRecord {
        seed: 999,
        ..results.trials[0].clone()
    });
    assert!(matches!(
        rebuild(added).validate(),
        Err(ResultsError::UnexpectedTrial { seed: 999 })
    ));

    let mut swapped = results.trials.clone();
    swapped.swap(0, 1);
    assert!(matches!(
        rebuild(swapped).validate(),
        Err(ResultsError::MissingTrial { seed: 100 })
    ));

    let mut replaced = results.trials.clone();
    replaced[2].seed = 555;
    assert!(matches!(
        rebuild(replaced).validate(),
        Err(ResultsError::UnexpectedTrial { seed: 555 })
    ));

    let mut short = results.trials.clone();
    short[0].counts.attempted = CASES - 1;
    assert!(matches!(
        rebuild(short).validate(),
        Err(ResultsError::UnequalBudget {
            seed: 100,
            found: 47,
            expected: 48
        })
    ));
    let mut over = results.trials.clone();
    over[0].counts.attempted = CASES + 1;
    over[0].outcome = TrialOutcome::HarnessFailure {
        message: "over".into(),
    };
    assert!(
        matches!(
            rebuild(over).validate(),
            Err(ResultsError::UnequalBudget {
                seed: 100,
                found: 49,
                expected: 48
            })
        ),
        "a harness failure never reaches past the budget"
    );

    let mut edited = results.clone();
    edited.summary.trials = 5;
    assert!(matches!(
        edited.validate(),
        Err(ResultsError::SummaryMismatch)
    ));
    let mut flattered = results.clone();
    let eligible = flattered
        .summary
        .distributions
        .get_mut(&Metric::Eligible)
        .expect("sampled");
    eligible.median = eligible.maximum.saturating_add(1);
    assert!(matches!(
        flattered.validate(),
        Err(ResultsError::SummaryMismatch)
    ));

    let mut other_schema = results;
    other_schema.schema_version = EVALUATION_SCHEMA_VERSION + 1;
    assert!(matches!(
        other_schema.validate(),
        Err(ResultsError::Version {
            found: 2,
            supported: 1
        })
    ));
}

#[test]
fn distributions_use_nearest_rank_quantiles() {
    let distribution = Distribution::of([5, 1, 3, 2, 4]).expect("five samples");
    assert_eq!(distribution.samples, [1, 2, 3, 4, 5]);
    assert_eq!(
        (
            distribution.minimum,
            distribution.lower_quartile,
            distribution.median,
            distribution.upper_quartile,
            distribution.maximum,
            distribution.total,
        ),
        (1, 2, 3, 4, 5, 15)
    );
    let even = Distribution::of([10, 20, 30, 40]).expect("four samples");
    assert_eq!(
        (even.lower_quartile, even.median, even.upper_quartile),
        (10, 20, 30)
    );
    let one = Distribution::of([7]).expect("one sample");
    assert_eq!((one.minimum, one.median, one.maximum), (7, 7, 7));
    assert_eq!(Distribution::of([]), None);
    let saturated = Distribution::of([u64::MAX, 1]).expect("two samples");
    assert_eq!(saturated.total, u64::MAX);
}

#[test]
fn superiority_is_the_exact_probability_that_a_candidate_sample_wins() {
    let apart = Superiority::of(&[3, 4], &[1, 2]).expect("samples");
    assert_eq!(
        apart,
        Superiority {
            favourable: 8,
            pairs: 8
        }
    );
    assert_eq!(apart.favours(), Ordering::Greater);
    assert_eq!(apart.magnitude(), Magnitude::Large);

    let behind = Superiority::of(&[1, 2], &[3, 4]).expect("samples");
    assert_eq!(
        behind,
        Superiority {
            favourable: 0,
            pairs: 8
        }
    );
    assert_eq!(behind.favours(), Ordering::Less);
    assert_eq!(behind.magnitude(), Magnitude::Large);

    let same = Superiority::of(&[1, 2, 3], &[1, 2, 3]).expect("samples");
    assert_eq!(
        same,
        Superiority {
            favourable: 9,
            pairs: 18
        }
    );
    assert_eq!(same.favours(), Ordering::Equal);
    assert_eq!(same.magnitude(), Magnitude::Negligible);

    assert_eq!(Superiority::of(&[], &[1]), None);
    assert_eq!(Superiority::of(&[1], &[]), None);
}

#[test]
fn magnitude_bands_sit_at_the_conventional_thresholds() {
    for (favourable, expected) in [
        (110, Magnitude::Negligible),
        (111, Magnitude::Negligible),
        (112, Magnitude::Small),
        (127, Magnitude::Small),
        (128, Magnitude::Medium),
        (141, Magnitude::Medium),
        (142, Magnitude::Large),
        (200, Magnitude::Large),
        (88, Magnitude::Small),
        (72, Magnitude::Medium),
        (58, Magnitude::Large),
        (0, Magnitude::Large),
    ] {
        let superiority = Superiority {
            favourable,
            pairs: 200,
        };
        assert_eq!(superiority.magnitude(), expected, "{favourable}/200");
    }
}

/// `ori 0,0,0`: decodes and stays eligible, and reaches nothing else.
const PPU_NOP: u32 = 0x6000_0000;
/// Opcode zero: the decoder refuses it.
const PPU_UNDECODABLE: u32 = 0;

/// The structured plan's trials with every generated word replaced by `word`.
fn degraded(word: u32) -> EvaluationResults {
    let plan = plan(FuzzTarget::PpuInstruction, GenerationStrategy::Structured);
    let trials = plan
        .seeds
        .iter()
        .map(|seed| {
            run_trial_with(&plan, *seed, |config| {
                crate::ppu::run_instructions_with(config, Some(&[word]))
            })
        })
        .collect();
    EvaluationResults::from_trials(plan, environment(), trials).expect("complete results")
}

fn metric(comparison: &Comparison, metric: Metric) -> &MetricComparison {
    comparison
        .metrics
        .iter()
        .find(|compared| compared.metric == metric)
        .unwrap_or_else(|| panic!("{metric:?} was not compared"))
}

#[test]
fn a_worse_generator_is_not_ranked_better() {
    let structured = results(plan(
        FuzzTarget::PpuInstruction,
        GenerationStrategy::Structured,
    ));
    let nothing = degraded(PPU_UNDECODABLE);
    assert!(nothing
        .trials
        .iter()
        .all(|trial| trial.outcome == TrialOutcome::NoEligibleCases));
    let nop_only = degraded(PPU_NOP);
    assert!(nop_only
        .trials
        .iter()
        .all(|trial| trial.counts.eligible == CASES && trial.counts.instruction_kinds == 1));
    assert!(
        distribution(&nop_only, Metric::Eligible).minimum
            > distribution(&structured, Metric::Eligible).maximum,
        "the single-word generator must look better on eligibility alone"
    );

    let worse = compare(&structured, &nothing).expect("comparable");
    assert_eq!(worse.trials, u64::from(TRIALS));
    assert_eq!(worse.cases_per_trial, CASES);
    assert_eq!(worse.verdict(), ComparisonVerdict::Regressed);
    for regressed in [
        Metric::Eligible,
        Metric::ExecutedSteps,
        Metric::InstructionKinds,
        Metric::MetamorphicExecutions,
    ] {
        assert!(worse.regressions().contains(&regressed), "{regressed:?}");
        let compared = metric(&worse, regressed);
        assert_eq!(compared.verdict, MetricVerdict::Regressed, "{regressed:?}");
        assert_eq!(compared.superiority.favours(), Ordering::Less);
        assert_eq!(compared.magnitude, Magnitude::Large);
    }
    let better = compare(&nothing, &structured).expect("comparable");
    assert_eq!(better.verdict(), ComparisonVerdict::Improved);
    assert!(better.regressions().is_empty());

    // A generator that makes every case eligible but reaches one kind wins
    // on eligibility and still ranks as a regression.
    let narrow = compare(&structured, &nop_only).expect("comparable");
    assert_eq!(
        metric(&narrow, Metric::Eligible).verdict,
        MetricVerdict::Improved
    );
    assert_eq!(narrow.verdict(), ComparisonVerdict::Regressed);
    for regressed in [
        Metric::InstructionKinds,
        Metric::EffectClasses,
        Metric::MetamorphicExecutions,
    ] {
        assert!(narrow.regressions().contains(&regressed), "{regressed:?}");
    }
    assert_ne!(
        compare(&nop_only, &structured)
            .expect("comparable")
            .verdict(),
        ComparisonVerdict::Improved,
        "a trade-off is never an improvement"
    );

    let same = compare(&structured, &structured).expect("comparable");
    assert_eq!(same.verdict(), ComparisonVerdict::Indistinguishable);
    assert!(same
        .metrics
        .iter()
        .all(|metric| metric.verdict == MetricVerdict::Indistinguishable));
    let json = serde_json::to_string(&worse).expect("serializes");
    assert_eq!(
        serde_json::from_str::<Comparison>(&json).expect("parses"),
        worse
    );
}

#[test]
fn comparisons_refuse_unequal_engines_budgets_trial_counts_and_incomplete_results() {
    let baseline = results(plan(
        FuzzTarget::SpuInstruction,
        GenerationStrategy::Structured,
    ));
    let other_engine = results(plan(
        FuzzTarget::SpuSequence,
        GenerationStrategy::Structured,
    ));
    assert!(matches!(
        compare(&baseline, &other_engine),
        Err(ComparisonError::DifferentTarget {
            baseline: FuzzTarget::SpuInstruction,
            candidate: FuzzTarget::SpuSequence,
        })
    ));
    let other_budget = results(EvaluationPlan {
        budget: TrialBudget { cases: CASES + 1 },
        ..plan(FuzzTarget::SpuInstruction, GenerationStrategy::Structured)
    });
    assert!(matches!(
        compare(&baseline, &other_budget),
        Err(ComparisonError::DifferentBudget {
            baseline: 48,
            candidate: 49
        })
    ));
    let longer_sequences = results(EvaluationPlan {
        sequence_words: 8,
        ..plan(FuzzTarget::SpuSequence, GenerationStrategy::Structured)
    });
    assert!(matches!(
        compare(&other_engine, &longer_sequences),
        Err(ComparisonError::DifferentSequenceWords {
            baseline: 6,
            candidate: 8
        })
    ));
    let longer_words_on_an_instruction_engine = results(EvaluationPlan {
        sequence_words: 8,
        ..plan(FuzzTarget::SpuInstruction, GenerationStrategy::Structured)
    });
    assert!(
        compare(&baseline, &longer_words_on_an_instruction_engine).is_ok(),
        "an instruction engine reads no sequence length"
    );
    let fewer = results(EvaluationPlan {
        seeds: vec![100, 101, 102],
        ..plan(FuzzTarget::SpuInstruction, GenerationStrategy::Structured)
    });
    assert!(matches!(
        compare(&baseline, &fewer),
        Err(ComparisonError::DifferentTrialCount {
            baseline: 6,
            candidate: 3
        })
    ));
    let mut tampered = baseline.clone();
    tampered.summary.trials = 1;
    assert!(matches!(
        compare(&baseline, &tampered),
        Err(ComparisonError::Invalid {
            side: ResultsSide::Candidate,
            source: ResultsError::SummaryMismatch
        })
    ));
    assert!(matches!(
        compare(&tampered, &baseline),
        Err(ComparisonError::Invalid {
            side: ResultsSide::Baseline,
            ..
        })
    ));
    assert_eq!(ResultsSide::Baseline.to_string(), "baseline");
    assert_eq!(ResultsSide::Candidate.to_string(), "candidate");
}

#[test]
fn seeded_defects_populate_findings_time_to_defect_and_reduction_cost() {
    // A small budget: every seeded case is a finding, and each one reduces.
    let plan = EvaluationPlan {
        budget: TrialBudget { cases: 8 },
        reduction: Some(ReductionRequest {
            policy: ReductionPolicy::Deterministic,
            budget: 24,
        }),
        ..plan(FuzzTarget::PpuInstruction, GenerationStrategy::Structured)
    };
    let clean = results(plan.clone());
    assert!(clean.trials.iter().all(|trial| trial.reduction
        == Some(ReductionCost {
            findings: 0,
            reduced: 0,
            irreducible: 0,
            failed: 0,
            evaluations: 0,
            words_before: 0,
            words_after: 0,
        })));
    assert!(!clean
        .summary
        .distributions
        .contains_key(&Metric::FirstFindingOffset));

    let _guard = seed(SeededDefect::IllegalOutcome);
    let seeded = results(plan);
    for trial in &seeded.trials {
        assert_eq!(trial.outcome, TrialOutcome::SemanticFinding);
        assert!(trial.finding_total() > 0);
        assert!(trial.unique_fingerprints > 0);
        assert!(trial.first_finding_offset.is_some_and(|offset| offset < 8));
        let cost = trial.reduction.expect("reduction requested");
        assert!(cost.findings > 0);
        assert_eq!(cost.reduced + cost.irreducible + cost.failed, cost.findings);
        assert!(cost.evaluations >= cost.findings);
        // An instruction finding reduces one case word, and no shrinker drops
        // a lone word: every finished finding counts one word on each side.
        assert_eq!(cost.words_before, cost.findings);
        assert_eq!(cost.words_after, cost.findings);
    }
    for metric in [
        Metric::Findings,
        Metric::UniqueFingerprints,
        Metric::FirstFindingOffset,
        Metric::ReductionEvaluations,
    ] {
        assert_eq!(
            distribution(&seeded, metric).samples.len(),
            TRIALS as usize,
            "{metric:?}"
        );
    }
    let comparison = compare(&clean, &seeded).expect("comparable");
    assert_eq!(
        comparison
            .metrics
            .iter()
            .find(|metric| metric.metric == Metric::Findings)
            .expect("findings compared")
            .verdict,
        MetricVerdict::Improved
    );
}

/// The report of a campaign whose harness failed after `reached` cases.
fn harness_failed_run(plan: &EvaluationPlan, reached: u64) -> FuzzRun {
    let mut report = FuzzReport::new(
        plan.target,
        plan.seeds[0],
        plan.strategy,
        RetentionConfig::default(),
        4,
        plan.sequence_words,
    );
    report.cases = reached;
    FuzzRun {
        outcome: RunOutcome::HarnessFailure(FuzzError::from(
            InvariantError::EmptyGeneratedSequence,
        )),
        report,
    }
}

#[test]
fn a_trial_the_harness_did_not_finish_is_recorded_under_budget_but_never_ranked() {
    let plan = plan(FuzzTarget::SpuInstruction, GenerationStrategy::Structured);
    let mut trials = plan
        .seeds
        .iter()
        .map(|seed| run_trial(&plan, *seed).expect("runs"))
        .collect::<Vec<_>>();
    trials[2] = run_trial_with(&plan, plan.seeds[2], |_| {
        harness_failed_run(&plan, CASES / 2)
    });
    assert_eq!(trials[2].counts.attempted, CASES / 2);
    let unfinished =
        EvaluationResults::from_trials(plan.clone(), environment(), trials).expect("assembles");
    assert!(unfinished.validate().is_ok());
    assert!(matches!(
        unfinished.trials[2].outcome,
        TrialOutcome::HarnessFailure { .. }
    ));
    let finished = results(plan);
    assert!(matches!(
        compare(&finished, &unfinished),
        Err(ComparisonError::HarnessFailed {
            side: ResultsSide::Candidate,
            seed: 102
        })
    ));
    assert!(matches!(
        compare(&unfinished, &finished),
        Err(ComparisonError::HarnessFailed {
            side: ResultsSide::Baseline,
            seed: 102
        })
    ));
}

#[test]
fn comparable_plans_are_checked_before_any_trial_exists() {
    let plan = plan(FuzzTarget::SpuSequence, GenerationStrategy::Structured);
    assert!(comparable(&plan, &plan).is_ok());
    let fewer = EvaluationPlan {
        seeds: vec![100, 101],
        ..plan.clone()
    };
    assert!(matches!(
        comparable(&plan, &fewer),
        Err(ComparisonError::DifferentTrialCount {
            baseline: 6,
            candidate: 2
        })
    ));
    let longer = EvaluationPlan {
        sequence_words: 7,
        ..plan.clone()
    };
    assert!(matches!(
        comparable(&plan, &longer),
        Err(ComparisonError::DifferentSequenceWords {
            baseline: 6,
            candidate: 7
        })
    ));
}

#[test]
fn wall_time_belongs_to_the_caller_and_is_summarized_over_the_trials_that_carry_it() {
    let plan = plan(FuzzTarget::SpuInstruction, GenerationStrategy::Structured);
    let mut trials = plan
        .seeds
        .iter()
        .map(|seed| run_trial(&plan, *seed).expect("runs"))
        .collect::<Vec<_>>();
    for (index, trial) in trials.iter_mut().enumerate() {
        trial.wall_ms = (index % 2 == 0).then_some(10 + index as u64);
    }
    let results = EvaluationResults::from_trials(plan, environment(), trials).expect("complete");
    let wall = distribution(&results, Metric::WallMs);
    assert_eq!(wall.samples, [10, 12, 14]);
    assert_eq!(results.samples(Metric::WallMs), [10, 12, 14]);
}
