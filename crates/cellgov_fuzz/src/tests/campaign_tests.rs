use super::*;

use serde_json::json;

fn schedule(
    first: u64,
    count: u64,
    shard: CampaignShard,
    cancellation: Option<u64>,
) -> CampaignSchedule {
    CampaignSchedule {
        cases: CaseRange { first, count },
        shard,
        cancellation: cancellation.map(CancellationBoundary),
    }
}

fn indices(schedule: CampaignSchedule) -> Vec<u64> {
    schedule.case_indices().unwrap().collect()
}

#[test]
fn the_campaign_version_is_four() {
    assert_eq!(CAMPAIGN_VERSION, CampaignVersion(4));
}

#[test]
fn campaign_versions_display_as_bare_numbers() {
    assert_eq!(CampaignVersion(4).to_string(), "4");
    assert_eq!(CampaignVersion(u32::MAX).to_string(), "4294967295");
}

#[test]
fn campaign_versions_serialize_transparently_and_order_numerically() {
    assert_eq!(serde_json::to_value(CampaignVersion(7)).unwrap(), json!(7));
    assert_eq!(
        serde_json::from_value::<CampaignVersion>(json!(u32::MAX)).unwrap(),
        CampaignVersion(u32::MAX)
    );
    assert_eq!(
        serde_json::from_value::<CampaignVersion>(json!(-1))
            .unwrap_err()
            .to_string(),
        "invalid value: integer `-1`, expected u32"
    );
    assert!(CampaignVersion(3) < CampaignVersion(4));
}

#[test]
fn generation_strategy_defaults_to_raw_words() {
    assert_eq!(GenerationStrategy::default(), GenerationStrategy::RawWords);
}

#[test]
fn generation_strategies_serialize_in_snake_case() {
    assert_eq!(
        serde_json::to_value(GenerationStrategy::Structured).unwrap(),
        json!("structured")
    );
    assert_eq!(
        serde_json::to_value(GenerationStrategy::RawWords).unwrap(),
        json!("raw_words")
    );
    assert_eq!(
        serde_json::from_value::<GenerationStrategy>(json!("raw_words")).unwrap(),
        GenerationStrategy::RawWords
    );
    assert_eq!(
        serde_json::from_value::<GenerationStrategy>(json!("Structured"))
            .unwrap_err()
            .to_string(),
        "unknown variant `Structured`, expected `structured` or `raw_words`"
    );
}

#[test]
fn structured_generation_orders_before_raw_words() {
    assert!(GenerationStrategy::Structured < GenerationStrategy::RawWords);
}

#[test]
fn the_whole_campaign_shard_is_member_zero_of_one() {
    assert_eq!(CampaignShard::ALL, CampaignShard { index: 0, count: 1 });
}

#[test]
fn the_default_schedule_runs_one_million_cases_from_zero_on_one_runner() {
    assert_eq!(
        CampaignSchedule::default(),
        schedule(0, 1_000_000, CampaignShard::ALL, None)
    );
}

#[test]
fn schedules_serialize_with_a_stable_key_order() {
    let encoded = concat!(
        "{\"cases\":{\"first\":0,\"count\":1000000},",
        "\"shard\":{\"index\":0,\"count\":1},\"cancellation\":null}"
    );

    assert_eq!(
        serde_json::to_string(&CampaignSchedule::default()).unwrap(),
        encoded
    );
    assert_eq!(
        serde_json::from_str::<CampaignSchedule>(encoded).unwrap(),
        CampaignSchedule::default()
    );
}

#[test]
fn a_cancellation_boundary_serializes_as_its_offset() {
    let encoded = serde_json::to_value(schedule(1, 2, CampaignShard::ALL, Some(2))).unwrap();

    assert_eq!(encoded["cancellation"], json!(2));
    assert_eq!(
        serde_json::from_value::<CancellationBoundary>(json!(u64::MAX)).unwrap(),
        CancellationBoundary(u64::MAX)
    );
}

#[test]
fn case_ranges_refuse_unknown_fields() {
    let error = serde_json::from_value::<CaseRange>(json!({ "first": 0, "count": 1, "extra": 0 }))
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "unknown field `extra`, expected `first` or `count`"
    );
}

#[test]
fn campaign_shards_refuse_unknown_fields() {
    let error =
        serde_json::from_value::<CampaignShard>(json!({ "index": 0, "count": 1, "extra": 0 }))
            .unwrap_err();

    assert_eq!(
        error.to_string(),
        "unknown field `extra`, expected `index` or `count`"
    );
}

#[test]
fn schedules_refuse_unknown_fields() {
    let error = serde_json::from_value::<CampaignSchedule>(json!({
        "cases": { "first": 0, "count": 1 },
        "shard": { "index": 0, "count": 1 },
        "cancellation": null,
        "extra": 0,
    }))
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "unknown field `extra`, expected one of `cases`, `shard`, `cancellation`"
    );
}

#[test]
fn zero_count_is_refused_before_the_shard_is_checked() {
    let schedule = schedule(0, 0, CampaignShard { index: 0, count: 0 }, None);

    assert_eq!(
        schedule.case_indices().unwrap_err(),
        ConfigurationError::ZeroIterations
    );
}

#[test]
fn a_range_ending_at_the_maximum_index_is_accepted() {
    assert_eq!(
        indices(schedule(u64::MAX - 1, 2, CampaignShard::ALL, None)),
        [u64::MAX - 1, u64::MAX]
    );
}

#[test]
fn a_range_ending_one_past_the_maximum_index_is_refused() {
    assert_eq!(
        schedule(u64::MAX - 1, 3, CampaignShard::ALL, None)
            .case_indices()
            .unwrap_err(),
        ConfigurationError::CaseRangeOverflow {
            first: u64::MAX - 1,
            count: 3,
        }
    );
}

#[test]
fn a_shard_selects_every_count_th_offset_from_its_index() {
    assert_eq!(
        indices(schedule(10, 8, CampaignShard { index: 1, count: 3 }, None)),
        [11, 14, 17]
    );
}

#[test]
fn the_last_shard_of_a_partition_is_valid() {
    assert_eq!(
        indices(schedule(10, 8, CampaignShard { index: 2, count: 3 }, None)),
        [12, 15]
    );
}

#[test]
fn a_shard_whose_first_offset_lies_beyond_the_range_yields_nothing() {
    let shard = CampaignShard {
        index: u32::MAX - 1,
        count: u32::MAX,
    };
    let mut cases = schedule(0, 3, shard, None).case_indices().unwrap();

    assert_eq!(cases.size_hint(), (0, Some(3)));
    assert_eq!(cases.next(), None);
    assert_eq!(cases.size_hint(), (0, Some(0)));
}

#[test]
fn a_shard_reaching_the_maximum_index_stops_without_wrapping() {
    assert_eq!(
        indices(schedule(
            u64::MAX - 1,
            2,
            CampaignShard { index: 1, count: 2 },
            None
        )),
        [u64::MAX]
    );
}

#[test]
fn size_hint_tracks_the_remaining_scheduled_offsets() {
    let mut cases = schedule(0, 5, CampaignShard::ALL, None)
        .case_indices()
        .unwrap();

    assert_eq!(cases.size_hint(), (0, Some(5)));
    assert_eq!(cases.next(), Some(0));
    assert_eq!(cases.size_hint(), (0, Some(4)));
}

#[test]
fn a_cancellation_at_the_count_keeps_the_whole_range() {
    let schedule = schedule(4, 2, CampaignShard::ALL, Some(2));

    assert_eq!(indices(schedule), [4, 5]);
    assert!(!schedule.is_cancelled());
}

#[test]
fn a_cancellation_at_zero_schedules_nothing_and_counts_as_cancelled() {
    let schedule = schedule(4, 2, CampaignShard::ALL, Some(0));

    assert_eq!(indices(schedule), [0u64; 0]);
    assert!(schedule.is_cancelled());
}

#[test]
fn an_absent_cancellation_is_not_cancelled() {
    assert!(!CampaignSchedule::default().is_cancelled());
}

#[test]
fn cancellation_applies_to_global_offsets_before_sharding() {
    assert_eq!(
        indices(schedule(
            100,
            5,
            CampaignShard { index: 1, count: 2 },
            Some(3)
        )),
        [101]
    );
}

#[test]
fn the_shard_is_checked_before_the_cancellation() {
    let schedule = schedule(0, 2, CampaignShard { index: 1, count: 1 }, Some(9));

    assert_eq!(
        schedule.case_indices().unwrap_err(),
        ConfigurationError::InvalidShard { index: 1, count: 1 }
    );
}

#[test]
fn new_replay_coordinates_carry_the_current_campaign_version() {
    let replay = ReplayCoordinates::new(
        FuzzTarget::SpuSequence,
        GenerationStrategy::RawWords,
        9,
        u64::MAX,
        3,
    );

    assert_eq!(
        replay,
        ReplayCoordinates {
            campaign_version: CAMPAIGN_VERSION,
            target: FuzzTarget::SpuSequence,
            strategy: GenerationStrategy::RawWords,
            seed: 9,
            case_index: u64::MAX,
            sequence_words: 3,
        }
    );
    assert_eq!(replay.validate(), Ok(()));
}

#[test]
fn replay_coordinates_serialize_with_a_stable_key_order() {
    let replay = ReplayCoordinates::new(
        FuzzTarget::SpuSequence,
        GenerationStrategy::RawWords,
        9,
        u64::MAX,
        3,
    );
    let encoded = concat!(
        r#"{"campaign_version":4,"target":"SpuSequence","strategy":"raw_words","#,
        r#""seed":9,"case_index":18446744073709551615,"sequence_words":3}"#
    );

    assert_eq!(serde_json::to_string(&replay).unwrap(), encoded);
    assert_eq!(
        serde_json::from_str::<ReplayCoordinates>(encoded).unwrap(),
        replay
    );
}

#[test]
fn replay_coordinates_refuse_unknown_fields() {
    let error = serde_json::from_value::<ReplayCoordinates>(json!({
        "campaign_version": 4,
        "target": "PpuInstruction",
        "strategy": "structured",
        "seed": 7,
        "case_index": 0,
        "sequence_words": 1,
        "extra": 0,
    }))
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        concat!(
            "unknown field `extra`, expected one of `campaign_version`, `target`, ",
            "`strategy`, `seed`, `case_index`, `sequence_words`"
        )
    );
}

#[test]
fn replay_coordinates_require_a_seed() {
    let error = serde_json::from_value::<ReplayCoordinates>(json!({
        "campaign_version": 4,
        "target": "PpuInstruction",
        "strategy": "structured",
        "case_index": 0,
        "sequence_words": 1,
    }))
    .unwrap_err();

    assert_eq!(error.to_string(), "missing field `seed`");
}

#[test]
fn version_one_replay_coordinates_keep_an_explicit_strategy() {
    let replay = serde_json::from_value::<ReplayCoordinates>(json!({
        "campaign_version": 1,
        "target": "PpuInstruction",
        "strategy": "structured",
        "seed": 7,
        "case_index": 0,
        "sequence_words": 1,
    }))
    .unwrap();

    assert_eq!(replay.strategy, GenerationStrategy::Structured);
}

#[test]
fn version_two_replay_coordinates_require_a_strategy() {
    let error = serde_json::from_value::<ReplayCoordinates>(json!({
        "campaign_version": 2,
        "target": "PpuInstruction",
        "seed": 7,
        "case_index": 0,
        "sequence_words": 1,
    }))
    .unwrap_err();

    assert_eq!(error.to_string(), "missing field `strategy`");
}

#[test]
fn replay_validation_refuses_every_other_version() {
    for found in [
        CampaignVersion(0),
        CampaignVersion(CAMPAIGN_VERSION.0 - 1),
        CampaignVersion(u32::MAX),
    ] {
        let replay = ReplayCoordinates {
            campaign_version: found,
            ..ReplayCoordinates::new(
                FuzzTarget::PpuInstruction,
                GenerationStrategy::Structured,
                1,
                0,
                1,
            )
        };

        assert_eq!(
            replay.validate(),
            Err(ReplayVersionError {
                found,
                supported: CAMPAIGN_VERSION,
            })
        );
    }
}
