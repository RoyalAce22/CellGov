use super::*;

#[test]
fn overflowing_case_ranges_are_typed_refusals() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: u64::MAX - 1,
                count: 4,
            },
            ..CampaignSchedule::default()
        },
        ..FuzzConfig::default()
    };

    assert_eq!(
        config.case_indices().unwrap_err(),
        ConfigurationError::CaseRangeOverflow {
            first: u64::MAX - 1,
            count: 4,
        }
    );
}

#[test]
fn the_maximum_case_index_is_replayable() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: u64::MAX,
                count: 1,
            },
            ..CampaignSchedule::default()
        },
        ..FuzzConfig::default()
    };

    assert_eq!(
        config.case_indices().unwrap().collect::<Vec<_>>(),
        [u64::MAX]
    );
}

#[test]
fn replay_version_mismatch_is_a_typed_refusal() {
    let coordinates = ReplayCoordinates {
        campaign_version: CampaignVersion(CAMPAIGN_VERSION.0 + 1),
        target: FuzzTarget::PpuSequence,
        strategy: GenerationStrategy::Structured,
        seed: 7,
        case_index: u64::MAX,
        sequence_words: 4,
    };

    assert_eq!(
        coordinates.validate(),
        Err(ReplayVersionError {
            found: CampaignVersion(CAMPAIGN_VERSION.0 + 1),
            supported: CAMPAIGN_VERSION,
        })
    );
}

#[test]
fn version_one_artifacts_parse_as_raw_words_then_receive_a_typed_refusal() {
    let config: FuzzConfig = serde_json::from_value(serde_json::json!({
        "campaign_version": 1,
        "seed": 7,
        "schedule": {
            "cases": { "first": 0, "count": 1 },
            "shard": { "index": 0, "count": 1 },
            "cancellation": null,
        },
        "max_findings": 1,
        "sequence_words": 1,
    }))
    .unwrap();
    let replay: ReplayCoordinates = serde_json::from_value(serde_json::json!({
        "campaign_version": 1,
        "target": "PpuInstruction",
        "seed": 7,
        "case_index": 0,
        "sequence_words": 1,
    }))
    .unwrap();

    assert_eq!(config.strategy, GenerationStrategy::RawWords);
    assert_eq!(replay.strategy, GenerationStrategy::RawWords);
    assert!(matches!(
        ppu::run_instructions(config).outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(
            ConfigurationError::UnsupportedCampaignVersion { .. }
        ))
    ));
    assert!(matches!(replay.validate(), Err(ReplayVersionError { .. })));
}

#[test]
fn version_three_artifacts_require_an_explicit_generation_strategy() {
    let config = serde_json::from_value::<FuzzConfig>(serde_json::json!({
        "campaign_version": 3,
        "seed": 7,
        "schedule": {
            "cases": { "first": 0, "count": 1 },
            "shard": { "index": 0, "count": 1 },
            "cancellation": null,
        },
        "max_findings": 1,
        "sequence_words": 1,
    }));
    let replay = serde_json::from_value::<ReplayCoordinates>(serde_json::json!({
        "campaign_version": 3,
        "target": "PpuInstruction",
        "seed": 7,
        "case_index": 0,
        "sequence_words": 1,
    }));

    assert!(config.is_err());
    assert!(replay.is_err());
}

#[test]
fn current_artifacts_require_an_explicit_retention_policy() {
    let config = serde_json::from_value::<FuzzConfig>(serde_json::json!({
        "campaign_version": 4,
        "seed": 7,
        "strategy": "structured",
        "schedule": {
            "cases": { "first": 0, "count": 1 },
            "shard": { "index": 0, "count": 1 },
            "cancellation": null,
        },
        "max_findings": 1,
        "sequence_words": 1,
    }));

    assert_eq!(config.unwrap_err().to_string(), "missing field `retention`");
}

#[test]
fn version_three_artifacts_default_the_new_retention_policy_then_receive_a_typed_refusal() {
    let config = serde_json::from_value::<FuzzConfig>(serde_json::json!({
        "campaign_version": 3,
        "seed": 7,
        "strategy": "structured",
        "schedule": {
            "cases": { "first": 0, "count": 1 },
            "shard": { "index": 0, "count": 1 },
            "cancellation": null,
        },
        "max_findings": 1,
        "sequence_words": 1,
    }))
    .unwrap();

    assert_eq!(config.retention, RetentionConfig::default());
    assert!(matches!(
        ppu::run_instructions(config).outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(
            ConfigurationError::UnsupportedCampaignVersion { .. }
        ))
    ));
}

#[test]
fn campaign_version_mismatch_is_a_typed_refusal() {
    let found = CampaignVersion(CAMPAIGN_VERSION.0 + 1);
    let run = ppu::run_instructions(FuzzConfig {
        campaign_version: found,
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(
            ConfigurationError::UnsupportedCampaignVersion {
                found: actual,
                supported: CAMPAIGN_VERSION,
            }
        )) if actual == found
    ));
    assert!(matches!(
        FuzzConfig {
            campaign_version: found,
            ..FuzzConfig::default()
        }
        .case_indices(),
        Err(ConfigurationError::UnsupportedCampaignVersion { .. })
    ));
}

#[test]
fn inapplicable_cases_are_classified_without_becoming_findings() {
    let mut report = FuzzReport::new(
        FuzzTarget::PpuInstruction,
        7,
        GenerationStrategy::Structured,
        RetentionConfig::default(),
        4,
        1,
    );
    report
        .assessed(&CaseAssessment::new(
            CaseEligibility::Unsupported,
            EligibilityReason::UnmetStatePrecondition,
            [CaseFeature::MappedMemory],
        ))
        .unwrap();
    report
        .assessed(&CaseAssessment::new(
            CaseEligibility::Undefined,
            EligibilityReason::ArchitecturallyUndefined,
            [],
        ))
        .unwrap();

    assert_eq!(report.eligibility_rate(), Some((0, 2)));
    assert!(report.findings.is_empty());
    assert_eq!(
        FuzzRun::completed(report).outcome,
        RunOutcome::UnsupportedAndUndefinedCases
    );
}

#[test]
fn legacy_inapplicability_findings_do_not_become_clean_completion() {
    for (kind, expected) in [
        (FindingKind::Unsupported, RunOutcome::UnsupportedCase),
        (FindingKind::Undefined, RunOutcome::UndefinedCase),
    ] {
        let mut report = FuzzReport::new(
            FuzzTarget::PpuInstruction,
            7,
            GenerationStrategy::Structured,
            RetentionConfig::default(),
            4,
            1,
        );
        report.finding_counts.insert(kind, 1);

        assert_eq!(FuzzRun::completed(report).outcome, expected);
    }
}

#[test]
fn zero_case_campaign_is_a_harness_failure() {
    let run = ppu::run_instructions(FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange { first: 0, count: 0 },
            ..CampaignSchedule::default()
        },
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(ConfigurationError::ZeroIterations))
    ));
}

#[test]
fn invalid_retention_policy_is_a_typed_configuration_failure() {
    let run = ppu::run_instructions(FuzzConfig {
        retention: RetentionConfig {
            capacity: 0,
            ..RetentionConfig::default()
        },
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(ConfigurationError::Retention {
            source: RetentionConfigError::ZeroCapacity,
        }))
    ));
    assert_eq!(run.report.cases, 0);
}

#[test]
fn shards_change_assignment_without_changing_case_identity() {
    let cases = CaseRange {
        first: 40,
        count: 8,
    };
    let whole = CampaignSchedule {
        cases,
        ..CampaignSchedule::default()
    }
    .case_indices()
    .unwrap()
    .collect::<Vec<_>>();
    let mut partitioned = Vec::new();
    for index in 0..3 {
        partitioned.extend(
            CampaignSchedule {
                cases,
                shard: CampaignShard { index, count: 3 },
                cancellation: None,
            }
            .case_indices()
            .unwrap(),
        );
    }
    partitioned.sort_unstable();

    assert_eq!(partitioned, whole);
}

#[test]
fn invalid_shards_are_typed_refusals() {
    for shard in [
        CampaignShard { index: 0, count: 0 },
        CampaignShard { index: 2, count: 2 },
    ] {
        let schedule = CampaignSchedule {
            shard,
            ..CampaignSchedule::default()
        };

        assert_eq!(
            schedule.case_indices().unwrap_err(),
            ConfigurationError::InvalidShard {
                index: shard.index,
                count: shard.count,
            }
        );
    }
}

#[test]
fn cancellation_stops_at_a_stable_global_boundary() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 100,
                count: 5,
            },
            shard: CampaignShard::ALL,
            cancellation: Some(CancellationBoundary(2)),
        },
        ..FuzzConfig::default()
    };

    assert_eq!(
        config.case_indices().unwrap().collect::<Vec<_>>(),
        [100, 101]
    );
    let run = ppu::run_instructions(config);
    assert_eq!(run.outcome, RunOutcome::Cancelled);
    assert_eq!(run.report.cases, 2);
}

#[test]
fn cancellation_beyond_the_range_is_a_typed_refusal() {
    let schedule = CampaignSchedule {
        cases: CaseRange { first: 4, count: 2 },
        cancellation: Some(CancellationBoundary(3)),
        ..CampaignSchedule::default()
    };

    assert_eq!(
        schedule.case_indices().unwrap_err(),
        ConfigurationError::CancellationOutOfRange {
            offset: 3,
            count: 2,
        }
    );
}

#[test]
fn campaign_and_replay_artifacts_round_trip() {
    let config = FuzzConfig {
        seed: 0x0123_4567_89ab_cdef,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: u64::MAX,
                count: 1,
            },
            ..CampaignSchedule::default()
        },
        sequence_words: 17,
        ..FuzzConfig::default()
    };
    let encoded = serde_json::to_string(&config).unwrap();
    let decoded: FuzzConfig = serde_json::from_str(&encoded).unwrap();
    let replay = ReplayCoordinates::new(
        FuzzTarget::PpuSequence,
        config.strategy,
        config.seed,
        u64::MAX,
        config.sequence_words,
    );
    let replay_encoded = serde_json::to_string(&replay).unwrap();
    let replay_decoded: ReplayCoordinates = serde_json::from_str(&replay_encoded).unwrap();

    assert_eq!(
        serde_json::to_value(config).unwrap(),
        serde_json::json!({
            "campaign_version": 4,
            "seed": 0x0123_4567_89ab_cdef_u64,
            "strategy": "structured",
            "schedule": {
                "cases": { "first": u64::MAX, "count": 1 },
                "shard": { "index": 0, "count": 1 },
                "cancellation": null,
            },
            "retention": {
                "capacity": 256,
                "per_kind_capacity": 8,
                "novelty_weight": 8,
                "rarity_weight": 4,
                "asymmetry_weight": 16,
                "policy": "balanced",
            },
            "max_findings": 20,
            "sequence_words": 17,
        })
    );
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::json!({
            "campaign_version": 4,
            "target": "PpuSequence",
            "strategy": "structured",
            "seed": 0x0123_4567_89ab_cdef_u64,
            "case_index": u64::MAX,
            "sequence_words": 17,
        })
    );
    assert_eq!(decoded, config);
    assert_eq!(replay_decoded, replay);
    assert_eq!(replay_decoded.validate(), Ok(()));
}

#[test]
fn parameter_stream_mutation_is_structural_and_bounds_checked() {
    let mut parameters = ParameterStream::new(vec![1, 2, 3]);

    assert_eq!(parameters.mutate(1, 9), Ok(()));
    assert_eq!(parameters.values(), [1, 9, 3]);
    assert_eq!(
        parameters.mutate(3, 0),
        Err(GeneratorError::ParameterIndex {
            index: 3,
            length: 3,
        })
    );
}

#[test]
fn a_target_runs_its_own_engine() {
    let config = FuzzConfig {
        schedule: CampaignSchedule {
            cases: CaseRange { first: 0, count: 2 },
            ..CampaignSchedule::default()
        },
        ..FuzzConfig::default()
    };
    assert_eq!(
        FuzzTarget::PpuInstruction.run(config),
        ppu::run_instructions(config)
    );
    assert_eq!(
        FuzzTarget::PpuSequence.run(config),
        ppu::run_sequences(config)
    );
    assert_eq!(
        FuzzTarget::SpuInstruction.run(config),
        spu::run_instructions(config)
    );
    assert_eq!(
        FuzzTarget::SpuSequence.run(config),
        spu::run_sequences(config)
    );
}

#[test]
fn only_a_sequence_target_generates_sequences() {
    assert!(FuzzTarget::PpuSequence.generates_sequences());
    assert!(FuzzTarget::SpuSequence.generates_sequences());
    assert!(!FuzzTarget::PpuInstruction.generates_sequences());
    assert!(!FuzzTarget::SpuInstruction.generates_sequences());
}

/// The library refuses a retention limit past its maximum on its own,
/// whatever a caller checked first.
#[test]
fn the_retained_finding_limit_holds_at_its_maximum_and_refuses_past_it() {
    let at = FuzzConfig {
        max_findings: MAX_RETAINED_FINDINGS,
        ..FuzzConfig::default()
    };
    assert!(at.validate_for_target(FuzzTarget::PpuInstruction).is_ok());
    let past = FuzzConfig {
        max_findings: MAX_RETAINED_FINDINGS + 1,
        ..FuzzConfig::default()
    };
    assert!(matches!(
        past.validate_for_target(FuzzTarget::PpuInstruction),
        Err(ConfigurationError::TooManyRetainedFindings {
            requested: 1_025,
            maximum: 1_024,
        })
    ));
}
