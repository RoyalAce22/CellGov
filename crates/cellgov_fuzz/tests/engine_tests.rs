//! Integration tests for the public fuzz-engine API.

use cellgov_effects::EffectKind;
use cellgov_fuzz::{
    ppu, spu, CampaignSchedule, CaseFeature, CaseRange, EligibilityReason, FuzzConfig,
    GenerationStrategy, InstructionIdentity, RunOutcome,
};
use cellgov_spu::instruction::SpuInstructionKind;

fn small_config() -> FuzzConfig {
    FuzzConfig {
        seed: 7,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 16,
            },
            ..CampaignSchedule::default()
        },
        max_findings: 4,
        sequence_words: 4,
        ..FuzzConfig::default()
    }
}

#[test]
fn engines_are_deterministic_library_calls() {
    let config = small_config();
    assert_eq!(ppu::run_instructions(config), ppu::run_instructions(config));
    assert_eq!(ppu::run_sequences(config), ppu::run_sequences(config));
    assert_eq!(spu::run_instructions(config), spu::run_instructions(config));
    assert_eq!(spu::run_sequences(config), spu::run_sequences(config));
}

#[test]
fn concurrent_runs_keep_independent_results() {
    let config = small_config();
    let expected = ppu::run_instructions(config);
    let workers = (0..4)
        .map(|_| std::thread::spawn(move || ppu::run_instructions(config)))
        .collect::<Vec<_>>();

    for worker in workers {
        assert_eq!(worker.join().expect("worker must not panic"), expected);
    }
}

#[test]
fn structured_instruction_campaigns_decode_every_generated_case() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 2_048,
            },
            ..CampaignSchedule::default()
        },
        ..small_config()
    };

    for run in [ppu::run_instructions(config), spu::run_instructions(config)] {
        assert!(!matches!(run.outcome, RunOutcome::HarnessFailure(_)));
        assert_eq!(run.report.cases, 2_048);
        assert_eq!(run.report.decoded, 2_048);
    }
}

#[test]
fn raw_word_campaigns_remain_a_separate_decoder_strategy() {
    let config = FuzzConfig {
        strategy: GenerationStrategy::RawWords,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 256,
            },
            ..CampaignSchedule::default()
        },
        ..small_config()
    };
    let ppu_run = ppu::run_instructions(config);
    let spu_run = spu::run_instructions(config);

    assert_eq!(ppu_run.report.strategy, GenerationStrategy::RawWords);
    assert_eq!(spu_run.report.strategy, GenerationStrategy::RawWords);
    assert!(ppu_run.report.decoded < ppu_run.report.cases);
    assert!(spu_run.report.decoded < spu_run.report.cases);
}

#[test]
fn raw_spu_undefined_encodings_are_refused_before_semantic_checks() {
    let config = FuzzConfig {
        seed: 0xA481_CE8D_0FF0_3E2A,
        strategy: GenerationStrategy::RawWords,
        schedule: CampaignSchedule {
            cases: CaseRange { first: 0, count: 1 },
            ..CampaignSchedule::default()
        },
        sequence_words: 1,
        ..small_config()
    };

    for run in [spu::run_instructions(config), spu::run_sequences(config)] {
        assert_eq!(run.outcome, RunOutcome::UndefinedCase);
        assert_eq!(run.report.cases, 1);
        assert_eq!(run.report.decoded, 1);
        assert_eq!(run.report.executed_steps, 1);
        assert_eq!(run.report.eligible_cases, 0);
        assert_eq!(run.report.undefined_cases, 1);
        assert_eq!(
            run.report
                .eligibility_reasons
                .get(&EligibilityReason::ArchitecturallyUndefined),
            Some(&1)
        );
        assert!(run.report.case_features.is_empty());
        assert!(run.report.finding_counts.is_empty());
        assert!(run.report.findings.is_empty());
    }
}

#[test]
fn raw_spu_unmodeled_channels_are_unsupported_not_named_faults() {
    let config = FuzzConfig {
        seed: 0x1D03_0CE2_EEA0_9F38,
        strategy: GenerationStrategy::RawWords,
        schedule: CampaignSchedule {
            cases: CaseRange { first: 0, count: 1 },
            ..CampaignSchedule::default()
        },
        sequence_words: 1,
        ..small_config()
    };

    for run in [spu::run_instructions(config), spu::run_sequences(config)] {
        assert_eq!(run.outcome, RunOutcome::UnsupportedCase);
        assert_eq!(run.report.cases, 1);
        assert_eq!(run.report.decoded, 1);
        assert_eq!(run.report.executed_steps, 1);
        assert_eq!(run.report.eligible_cases, 0);
        assert_eq!(run.report.unsupported_cases, 1);
        assert_eq!(run.report.undefined_cases, 0);
        assert_eq!(
            run.report
                .eligibility_reasons
                .get(&EligibilityReason::UnmodeledExecution),
            Some(&1)
        );
        assert!(run.report.case_features.is_empty());
        assert!(run.report.finding_counts.is_empty());
        assert!(run.report.findings.is_empty());
    }
}

#[test]
fn raw_spu_interrupt_options_without_modeled_execution_are_unsupported() {
    let config = FuzzConfig {
        seed: 0xA212_C9E1_694B_D553,
        strategy: GenerationStrategy::RawWords,
        schedule: CampaignSchedule {
            cases: CaseRange { first: 0, count: 1 },
            ..CampaignSchedule::default()
        },
        sequence_words: 1,
        ..small_config()
    };

    for run in [spu::run_instructions(config), spu::run_sequences(config)] {
        assert_eq!(run.outcome, RunOutcome::UnsupportedCase);
        assert_eq!(run.report.cases, 1);
        assert_eq!(run.report.decoded, 1);
        assert_eq!(run.report.executed_steps, 1);
        assert_eq!(run.report.eligible_cases, 0);
        assert_eq!(run.report.unsupported_cases, 1);
        assert_eq!(run.report.undefined_cases, 0);
        assert_eq!(
            run.report
                .eligibility_reasons
                .get(&EligibilityReason::UnmodeledExecution),
            Some(&1)
        );
        assert!(run.report.case_features.is_empty());
        assert!(run.report.finding_counts.is_empty());
        assert!(run.report.findings.is_empty());
    }
}

#[test]
fn structured_spu_constraints_retry_without_aborting_the_campaign() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 4_096,
            },
            ..CampaignSchedule::default()
        },
        ..small_config()
    };
    let run = spu::run_instructions(config);

    assert!(!matches!(
        run.outcome,
        RunOutcome::HarnessFailure(_) | RunOutcome::TargetPanic
    ));
    assert_eq!(run.report.cases, 4_096);
    assert_eq!(run.report.decoded, 4_096);
    for kind in [
        SpuInstructionKind::Bi,
        SpuInstructionKind::Bisl,
        SpuInstructionKind::Biz,
        SpuInstructionKind::Binz,
        SpuInstructionKind::Bihz,
        SpuInstructionKind::Bihnz,
        SpuInstructionKind::Hbr,
        SpuInstructionKind::Hbra,
        SpuInstructionKind::Hbrr,
    ] {
        assert!(
            run.report
                .instruction_kinds
                .contains(&InstructionIdentity::Spu(kind)),
            "structured campaign did not reach {kind:?}"
        );
    }
}

#[test]
fn state_aware_campaigns_report_eligibility_and_generation_features() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 512,
            },
            ..CampaignSchedule::default()
        },
        sequence_words: 8,
        ..small_config()
    };

    let ppu_instructions = ppu::run_instructions(config);
    let spu_instructions = spu::run_instructions(config);
    for run in [&ppu_instructions, &spu_instructions] {
        let classified =
            run.report.eligible_cases + run.report.unsupported_cases + run.report.undefined_cases;
        assert_eq!(classified, run.report.decoded);
        assert_eq!(run.report.eligibility_rate().map(|rate| rate.1), Some(512));
        assert!(run.report.eligible_cases > 0);
        assert_eq!(run.report.executed_steps, 512);
        assert_eq!(run.report.max_executed_depth, 1);
        assert!(run
            .report
            .case_features
            .contains_key(&CaseFeature::MappedMemory));
        assert!(run
            .report
            .case_features
            .contains_key(&CaseFeature::Reservation));
    }
    assert!(spu_instructions
        .report
        .case_features
        .contains_key(&CaseFeature::ChannelState));
}

#[test]
fn structured_sequences_expose_dependencies_and_deeper_execution_than_raw_words() {
    let structured = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 256,
            },
            ..CampaignSchedule::default()
        },
        sequence_words: 8,
        ..small_config()
    };
    let raw = FuzzConfig {
        strategy: GenerationStrategy::RawWords,
        ..structured
    };

    for (structured_run, raw_run) in [
        (ppu::run_sequences(structured), ppu::run_sequences(raw)),
        (spu::run_sequences(structured), spu::run_sequences(raw)),
    ] {
        assert_eq!(
            structured_run.report.eligible_cases
                + structured_run.report.unsupported_cases
                + structured_run.report.undefined_cases,
            256
        );
        assert!(structured_run.report.eligible_cases > 0);
        assert!(structured_run
            .report
            .case_features
            .contains_key(&CaseFeature::DependencyChain));
        assert!(!raw_run
            .report
            .case_features
            .contains_key(&CaseFeature::DependencyChain));
        assert!(structured_run.report.max_executed_depth >= 7);
        assert!(structured_run.report.executed_steps > raw_run.report.executed_steps);
    }
}

#[test]
fn dependency_chain_metrics_require_two_generated_consumers() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 256,
            },
            ..CampaignSchedule::default()
        },
        sequence_words: 2,
        ..small_config()
    };

    let run = spu::run_sequences(config);
    let dependency_cases = run
        .report
        .case_features
        .get(&CaseFeature::DependencyChain)
        .copied()
        .unwrap_or(0);

    // This seed selects a candidate chain for 192 cases. A feature needs two
    // generated consumers.
    assert!(dependency_cases > 0);
    assert!(dependency_cases < 192);
}

#[test]
fn structured_state_bias_reaches_effect_classes_with_named_preconditions() {
    let structured = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 8_192,
            },
            ..CampaignSchedule::default()
        },
        ..small_config()
    };
    let raw = FuzzConfig {
        strategy: GenerationStrategy::RawWords,
        ..structured
    };
    let ppu_run = ppu::run_instructions(structured);
    let spu_run = spu::run_instructions(structured);
    let raw_ppu = ppu::run_instructions(raw);
    let raw_spu = spu::run_instructions(raw);

    for feature in [CaseFeature::MappedMemory, CaseFeature::Reservation] {
        assert!(ppu_run.report.case_features.contains_key(&feature));
        assert!(spu_run.report.case_features.contains_key(&feature));
        assert!(!raw_ppu.report.case_features.contains_key(&feature));
        assert!(!raw_spu.report.case_features.contains_key(&feature));
    }
    assert!(spu_run
        .report
        .case_features
        .contains_key(&CaseFeature::ChannelState));
    for effect in [
        EffectKind::SharedReadIntent,
        EffectKind::SharedWriteIntent,
        EffectKind::ReservationAcquire,
        EffectKind::ClockRead,
    ] {
        assert!(
            ppu_run.report.effect_classes.contains_key(&effect),
            "structured PPU campaign missed {effect:?}"
        );
    }
    for effect in [
        EffectKind::MailboxReceiveAttempt,
        EffectKind::DmaEnqueue,
        EffectKind::ConditionalStore,
    ] {
        assert!(
            spu_run.report.effect_classes.contains_key(&effect),
            "structured SPU campaign missed {effect:?}"
        );
    }
}
