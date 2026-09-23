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
        config.seed,
        u64::MAX,
        config.sequence_words,
    );
    let replay_encoded = serde_json::to_string(&replay).unwrap();
    let replay_decoded: ReplayCoordinates = serde_json::from_str(&replay_encoded).unwrap();

    assert_eq!(
        serde_json::to_value(config).unwrap(),
        serde_json::json!({
            "campaign_version": 1,
            "seed": 0x0123_4567_89ab_cdef_u64,
            "schedule": {
                "cases": { "first": u64::MAX, "count": 1 },
                "shard": { "index": 0, "count": 1 },
                "cancellation": null,
            },
            "max_findings": 20,
            "sequence_words": 17,
        })
    );
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::json!({
            "campaign_version": 1,
            "target": "PpuSequence",
            "seed": 0x0123_4567_89ab_cdef_u64,
            "case_index": u64::MAX,
            "sequence_words": 17,
        })
    );
    assert_eq!(decoded, config);
    assert_eq!(replay_decoded, replay);
    assert_eq!(replay_decoded.validate(), Ok(()));
}
