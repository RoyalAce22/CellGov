use super::*;

use crate::report::{CheckIdentity, DivergenceClass, FindingKind, FuzzReport, SemanticFingerprint};
use crate::{
    CampaignVersion, FuzzError, GenerationStrategy, InvariantError, ReductionOutcome,
    ReplayCoordinates, RetentionConfig, CAMPAIGN_VERSION,
};

const REQUEST: ReductionRequest = ReductionRequest {
    policy: ReductionPolicy::Deterministic,
    budget: DEFAULT_REDUCTION_BUDGET,
};

/// Bit-level shrinker over any word list, independent of an interpreter.
fn bit_shrink(words: &[u32]) -> Vec<ReductionCandidate> {
    let mut candidates = Vec::new();
    if words.len() > 1 {
        for index in 0..words.len() {
            let mut shorter = words.to_vec();
            shorter.remove(index);
            candidates.push(ReductionCandidate {
                transform: ReductionTransform::DropWord { index },
                words: shorter,
            });
        }
    }
    for (index, &word) in words.iter().enumerate() {
        for bit in 0..u32::BITS {
            if word & (1 << bit) != 0 {
                let mut cleared = words.to_vec();
                cleared[index] = word & !(1 << bit);
                candidates.push(ReductionCandidate {
                    transform: ReductionTransform::ClearOperandBit { index, bit },
                    words: cleared,
                });
            }
        }
    }
    candidates
}

/// A modelled defect: two words, the first with bits 0 and 2 set.
fn model_verdict(words: &[u32]) -> Result<CandidateVerdict, ReductionError> {
    Ok(match words {
        [first, ..] if words.len() >= 2 && first & 0b101 == 0b101 => CandidateVerdict::Reproduced {
            finding_words: words.to_vec(),
        },
        [first, ..] if words.len() >= 2 && first & 0b101 == 0b100 => {
            CandidateVerdict::DifferentFinding
        }
        [_] => CandidateVerdict::Inapplicable,
        _ => CandidateVerdict::NoFinding,
    })
}

#[test]
fn fixpoint_reaches_the_minimal_case_and_keeps_the_finding() {
    let report = reduce(&[0b1111, 0b11, 0b1], REQUEST, bit_shrink, model_verdict).unwrap();
    assert_eq!(report.case_words, [0b101, 0]);
    assert_eq!(report.finding_words, [0b101, 0]);
    assert!(report.fixpoint);
    assert!(report.reduced());
    assert_eq!(report.applied.len(), 4);
    assert_eq!(report.rounds, 5);
    assert!(report.evaluations >= report.rounds + 2);
}

#[test]
fn candidates_that_cross_into_another_finding_or_undefined_behaviour_are_refused() {
    let mut evaluated = Vec::new();
    let report = reduce(&[0b101, 0], REQUEST, bit_shrink, |words| {
        evaluated.push(words.to_vec());
        model_verdict(words)
    })
    .unwrap();
    assert_eq!(report.case_words, [0b101, 0]);
    assert!(report.fixpoint);
    assert!(!report.reduced());
    assert_eq!(report.evaluations, 6);
    assert_eq!(
        evaluated,
        [
            vec![0b101, 0],
            vec![0],
            vec![0b101],
            vec![0b100, 0],
            vec![0b001, 0],
            vec![0b101, 0],
        ]
    );
}

#[test]
fn deterministic_policy_ignores_completion_order_and_greedy_follows_it() {
    let round = || {
        vec![
            ReductionCandidate {
                transform: ReductionTransform::DropWord { index: 1 },
                words: vec![0b111],
            },
            ReductionCandidate {
                transform: ReductionTransform::ClearOperandBit { index: 1, bit: 0 },
                words: vec![0b111, 0],
            },
        ]
    };
    let reproduced = || CandidateVerdict::Reproduced {
        finding_words: Vec::new(),
    };
    for (policy, expected) in [
        (ReductionPolicy::Deterministic, vec![0b111]),
        (ReductionPolicy::Greedy, vec![0b111, 0]),
    ] {
        let mut session =
            ReductionSession::new(&[0b111, 1], ReductionRequest { policy, ..REQUEST });
        session.verify(reproduced()).unwrap();
        session.begin_round(round()).unwrap();
        session
            .finish_round([(1, reproduced()), (0, reproduced())])
            .unwrap();
        assert_eq!(session.current(), expected, "{policy:?}");
    }
}

#[test]
fn a_shrinker_that_does_not_shrink_is_refused() {
    let error = reduce(
        &[0b101, 0],
        REQUEST,
        |words| {
            vec![ReductionCandidate {
                transform: ReductionTransform::DropWord { index: 0 },
                words: words.to_vec(),
            }]
        },
        model_verdict,
    )
    .unwrap_err();
    assert_eq!(
        error,
        ReductionError::InvalidCandidate {
            transform: "drop word"
        }
    );
}

#[test]
fn an_unreproduced_original_refuses_before_any_transform() {
    let mut shrunk = 0;
    let error = reduce(
        &[0b1, 0],
        REQUEST,
        |words| {
            shrunk += 1;
            bit_shrink(words)
        },
        model_verdict,
    )
    .unwrap_err();
    assert_eq!(error, ReductionError::OriginalNotReproduced);
    assert_eq!(shrunk, 0);
}

#[test]
fn the_budget_bounds_evaluations_and_reports_no_fixpoint() {
    let report = reduce(
        &[0b1111, 0b11, 0b1],
        ReductionRequest {
            budget: 3,
            ..REQUEST
        },
        bit_shrink,
        model_verdict,
    )
    .unwrap();
    assert!(!report.fixpoint);
    assert!(report.reduced());
    assert_eq!(report.rounds, 1);
    assert_eq!(report.case_words, [0b1111, 0b1]);
}

#[test]
fn a_budget_that_settles_no_round_is_a_failure_not_a_minimal_case() {
    let unsettled = reduce(
        &[0b1111, 0b11, 0b1],
        ReductionRequest {
            budget: 1,
            ..REQUEST
        },
        bit_shrink,
        model_verdict,
    )
    .unwrap();
    assert!(!unsettled.reduced() && !unsettled.fixpoint);
    assert_eq!(
        unsettled.into_outcome(),
        ReductionOutcome::Failed(ReductionError::BudgetExhausted { evaluations: 2 })
    );
    let partial = reduce(
        &[0b1111, 0b11, 0b1],
        ReductionRequest {
            budget: 3,
            ..REQUEST
        },
        bit_shrink,
        model_verdict,
    )
    .unwrap();
    assert_eq!(
        partial.into_outcome(),
        ReductionOutcome::Reduced(vec![0b1111, 0b1])
    );
    let minimal = reduce(&[0b101, 0], REQUEST, bit_shrink, model_verdict).unwrap();
    assert_eq!(minimal.into_outcome(), ReductionOutcome::Irreducible);
}

#[test]
fn a_changed_final_verdict_is_a_fingerprint_change() {
    let mut calls = 0;
    let error = reduce(
        &[0b101, 0],
        REQUEST,
        |_| Vec::new(),
        |words| {
            calls += 1;
            if calls == 2 {
                Ok(CandidateVerdict::NoFinding)
            } else {
                model_verdict(words)
            }
        },
    )
    .unwrap_err();
    assert_eq!(
        error,
        ReductionError::FingerprintChanged {
            transform: "no transform"
        }
    );
}

#[test]
fn the_session_refuses_steps_out_of_order() {
    let mut session = ReductionSession::new(&[1], REQUEST);
    assert!(matches!(
        session.begin_round(Vec::new()),
        Err(ReductionError::SessionOrder { .. })
    ));
    assert!(matches!(
        session.finish_round([]),
        Err(ReductionError::SessionOrder { .. })
    ));
    assert!(matches!(
        session.clone().finish(CandidateVerdict::NoFinding),
        Err(ReductionError::SessionOrder { .. })
    ));
    session
        .verify(CandidateVerdict::Reproduced {
            finding_words: vec![1],
        })
        .unwrap();
    assert!(matches!(
        session.verify(CandidateVerdict::NoFinding),
        Err(ReductionError::SessionOrder { .. })
    ));
}

#[test]
fn a_round_with_no_reported_verdict_is_not_a_fixpoint() {
    let mut session = ReductionSession::new(&[0b11], REQUEST);
    session
        .verify(CandidateVerdict::Reproduced {
            finding_words: vec![0b11],
        })
        .unwrap();
    let candidate = ReductionCandidate {
        transform: ReductionTransform::ClearOperandBit { index: 0, bit: 0 },
        words: vec![0b10],
    };
    session.begin_round(vec![candidate]).unwrap();
    assert_eq!(
        session.finish_round([]).unwrap_err(),
        ReductionError::SessionOrder {
            expected: "at least one verdict"
        }
    );
    assert!(!session.settled());
    session
        .finish_round([(0, CandidateVerdict::NoFinding)])
        .unwrap();
    assert!(session.settled());
}

#[test]
fn an_inapplicable_original_is_a_named_refusal() {
    let mut session = ReductionSession::new(&[1], REQUEST);
    assert_eq!(
        session.verify(CandidateVerdict::Inapplicable).unwrap_err(),
        ReductionError::OriginalInapplicable
    );
    let mut shrunk = 0;
    let error = reduce(
        &[0b101],
        REQUEST,
        |words| {
            shrunk += 1;
            bit_shrink(words)
        },
        model_verdict,
    )
    .unwrap_err();
    assert_eq!(error, ReductionError::OriginalInapplicable);
    assert_eq!(shrunk, 0);
}

fn config(strategy: GenerationStrategy) -> FuzzConfig {
    FuzzConfig {
        seed: 7,
        strategy,
        sequence_words: 4,
        ..FuzzConfig::default()
    }
}

const TARGETS: [FuzzTarget; 4] = [
    FuzzTarget::PpuInstruction,
    FuzzTarget::PpuSequence,
    FuzzTarget::SpuInstruction,
    FuzzTarget::SpuSequence,
];

#[test]
fn generated_words_replay_identically_through_evaluate_case() {
    for target in TARGETS {
        for strategy in [GenerationStrategy::Structured, GenerationStrategy::RawWords] {
            let config = config(strategy);
            let words = case_words(target, config, 3).unwrap();
            assert!(!words.is_empty(), "{target:?} {strategy:?}");
            let evaluated = evaluate_case(target, config, 3, &words);
            let baseline = match target {
                FuzzTarget::PpuInstruction => ppu::run_instructions(single_case(config, 3)),
                FuzzTarget::PpuSequence => ppu::run_sequences(single_case(config, 3)),
                FuzzTarget::SpuInstruction => spu::run_instructions(single_case(config, 3)),
                FuzzTarget::SpuSequence => spu::run_sequences(single_case(config, 3)),
            };
            assert_eq!(evaluated, baseline, "{target:?} {strategy:?}");
            assert_eq!(evaluated.report.cases, 1);
        }
    }
}

#[test]
fn substituted_words_change_the_evaluated_case() {
    let config = config(GenerationStrategy::Structured);
    let words = case_words(FuzzTarget::PpuSequence, config, 3).unwrap();
    let evaluated = evaluate_case(FuzzTarget::PpuSequence, config, 3, &words[..1]);
    assert_eq!(evaluated.report.cases, 1);
    assert_eq!(evaluated.report.decoded, 1);
    assert!(evaluated.report.max_executed_depth <= 1);
}

fn ppu_kind(raw: u32) -> cellgov_ppu::instruction::fuzz::PpuFuzzKind {
    cellgov_ppu::decode::decode(raw)
        .unwrap()
        .fuzz_descriptor(raw)
        .kind
}

fn spu_kind(raw: u32) -> cellgov_spu::instruction::SpuInstructionKind {
    cellgov_spu::instruction::SpuInstructionKind::from(cellgov_spu::decode::decode(raw).unwrap())
}

#[test]
fn shrink_candidates_keep_every_decoded_kind_and_shrink_strictly() {
    let mut nonempty = 0;
    for target in TARGETS {
        for case_index in 0..8 {
            let config = config(GenerationStrategy::Structured);
            let words = case_words(target, config, case_index).unwrap();
            let candidates = shrink_candidates(target, &words);
            let measure = ReductionMeasure::of(&words);
            for candidate in &candidates {
                assert!(ReductionMeasure::of(&candidate.words) < measure);
                match candidate.transform {
                    ReductionTransform::DropWord { index } => {
                        assert!(words.len() > 1);
                        let mut expected = words.clone();
                        expected.remove(index);
                        assert_eq!(candidate.words, expected);
                    }
                    ReductionTransform::ClearOperandBit { index, bit } => {
                        assert_eq!(candidate.words[index], words[index] & !(1 << bit));
                        assert_eq!(candidate.words.len(), words.len());
                        match target {
                            FuzzTarget::PpuInstruction | FuzzTarget::PpuSequence => {
                                assert_eq!(
                                    ppu_kind(candidate.words[index]),
                                    ppu_kind(words[index])
                                );
                            }
                            FuzzTarget::SpuInstruction | FuzzTarget::SpuSequence => {
                                assert_eq!(
                                    spu_kind(candidate.words[index]),
                                    spu_kind(words[index])
                                );
                            }
                        }
                    }
                }
            }
            nonempty += usize::from(!candidates.is_empty());
        }
    }
    assert!(
        nonempty >= 8,
        "shrinkers produced candidates for {nonempty} cases"
    );
}

fn synthetic_finding(target: FuzzTarget, words: Vec<u32>) -> Finding {
    Finding {
        fingerprint: SemanticFingerprint {
            target,
            instruction_kind: None,
            check: CheckIdentity::LegalOutcome,
            divergence: DivergenceClass::Outcome,
            outcome: None,
            effect: None,
        },
        kind: FindingKind::IllegalOutcome,
        replay: ReplayCoordinates {
            campaign_version: CAMPAIGN_VERSION,
            target,
            strategy: GenerationStrategy::Structured,
            seed: 7,
            case_index: 3,
            sequence_words: 4,
        },
        original_words: words,
        observation: None,
        reduction: ReductionOutcome::NotAttempted,
        panic_payload: None,
    }
}

#[test]
fn reduce_finding_refuses_a_finding_the_engine_no_longer_produces() {
    let config = config(GenerationStrategy::Structured);
    for target in TARGETS {
        let words = case_words(target, config, 3).unwrap();
        // The engine's own eligibility counters decide which refusal fits this
        // case: the reducer refuses an unsupported or undefined case by name.
        let baseline = evaluate_case(target, config, 3, &words).report;
        let expected = if baseline.unsupported_cases > 0 || baseline.undefined_cases > 0 {
            ReductionError::OriginalInapplicable
        } else {
            ReductionError::OriginalNotReproduced
        };
        let finding = synthetic_finding(target, words);
        assert_eq!(
            reduce_finding(config, &finding, REQUEST).unwrap_err(),
            expected,
            "{target:?}"
        );
    }
}

#[test]
fn an_inapplicable_candidate_is_refused_even_when_it_carries_the_finding() {
    let finding = synthetic_finding(FuzzTarget::PpuInstruction, vec![0x3860_0007]);
    let mut report = FuzzReport::new(
        FuzzTarget::PpuInstruction,
        7,
        GenerationStrategy::Structured,
        RetentionConfig::default(),
        4,
        4,
    );
    report.considered().unwrap();
    report.finding(finding.clone()).unwrap();
    assert_eq!(
        classify_run(&finding, &FuzzRun::completed(report.clone())).unwrap(),
        CandidateVerdict::Reproduced {
            finding_words: vec![0x3860_0007]
        }
    );
    for (unsupported, undefined) in [(1, 0), (0, 1)] {
        let mut inapplicable = report.clone();
        inapplicable.unsupported_cases = unsupported;
        inapplicable.undefined_cases = undefined;
        assert_eq!(
            classify_run(&finding, &FuzzRun::completed(inapplicable)).unwrap(),
            CandidateVerdict::Inapplicable,
            "unsupported={unsupported} undefined={undefined}"
        );
    }
}

#[test]
fn reduce_finding_refuses_incompatible_coordinates_and_empty_cases() {
    let config = config(GenerationStrategy::Structured);
    let finding = synthetic_finding(FuzzTarget::PpuInstruction, vec![0x3860_0007]);
    let mut other_seed = config;
    other_seed.seed += 1;
    assert_eq!(
        reduce_finding(other_seed, &finding, REQUEST).unwrap_err(),
        ReductionError::IncompatibleReplay
    );
    let mut other_version = finding.clone();
    other_version.replay.campaign_version = CampaignVersion(CAMPAIGN_VERSION.0 + 1);
    assert_eq!(
        reduce_finding(config, &other_version, REQUEST).unwrap_err(),
        ReductionError::IncompatibleReplay
    );
    let empty = synthetic_finding(FuzzTarget::PpuInstruction, Vec::new());
    assert_eq!(
        reduce_finding(config, &empty, REQUEST).unwrap_err(),
        ReductionError::NothingToReduce
    );
}

#[test]
fn classify_run_reports_identity_applicability_and_engine_failure() {
    let finding = synthetic_finding(FuzzTarget::PpuInstruction, vec![0x3860_0007]);
    let mut report = FuzzReport::new(
        FuzzTarget::PpuInstruction,
        7,
        GenerationStrategy::Structured,
        RetentionConfig::default(),
        4,
        4,
    );
    report.considered().unwrap();
    assert_eq!(
        classify_run(&finding, &FuzzRun::completed(report.clone())).unwrap(),
        CandidateVerdict::NoFinding
    );
    let mut different = finding.clone();
    different.fingerprint.divergence = DivergenceClass::ControlFlow;
    report.finding(different).unwrap();
    assert_eq!(
        classify_run(&finding, &FuzzRun::completed(report.clone())).unwrap(),
        CandidateVerdict::DifferentFinding
    );
    let mut same = finding.clone();
    same.original_words = vec![0x3860_0006];
    report.finding(same).unwrap();
    assert_eq!(
        classify_run(&finding, &FuzzRun::completed(report.clone())).unwrap(),
        CandidateVerdict::Reproduced {
            finding_words: vec![0x3860_0006]
        }
    );
    report.unsupported_cases = 1;
    let mut unsupported = report.clone();
    unsupported.findings.clear();
    assert_eq!(
        classify_run(&finding, &FuzzRun::completed(unsupported)).unwrap(),
        CandidateVerdict::Inapplicable
    );
    let failed = FuzzRun::failed(
        report,
        FuzzError::from(InvariantError::EmptyGeneratedSequence),
    );
    assert!(matches!(
        classify_run(&finding, &failed),
        Err(ReductionError::CandidateEvaluation { .. })
    ));
}
