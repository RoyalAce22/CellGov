//! A counted word domain identifies inputs without storing the full 32-bit space.

use cellgov_fuzz::raw_decode::{
    scan_raw_decoder, RawDecodeArtifact, RawDecodeDomain, RawDecodeError, RawDecodeStatus,
    RawDecoder, MAX_RAW_DECODE_CHUNK, RAW_DECODE_SCHEMA_VERSION,
};

#[test]
fn both_decoders_use_the_same_replayable_artifact_and_chunk_engine() {
    let domain = RawDecodeDomain::new(0, 263).expect("bounded word interval");
    for decoder in [RawDecoder::Ppu, RawDecoder::Spu] {
        let first = scan_raw_decoder(decoder, domain, 11, 3, None).expect("scan must finish");
        let second =
            scan_raw_decoder(decoder, domain, 17, 7, None).expect("other partition must finish");
        assert_eq!(first, second);
        assert_eq!(first.schema_version, RAW_DECODE_SCHEMA_VERSION);
        assert_eq!((first.domain.first, first.domain.count), (0, 263));
        assert_eq!(first.status, RawDecodeStatus::Complete);
        assert_eq!(first.processed, 263);
        assert_eq!(first.accepted + first.refused + first.panics, 263);
        assert!(first.is_clean());
        let mut incomplete = first.clone();
        incomplete.processed -= 1;
        assert!(!incomplete.is_clean());
        assert_eq!(first.word_at(0), Some(0));
        assert_eq!(first.word_at(262), Some(262));
        assert_eq!(first.word_at(263), None);
        let json = serde_json::to_string(&first).expect("artifact must serialize");
        let restored =
            RawDecodeArtifact::parse_json(&json).expect("artifact must replay from JSON");
        assert_eq!(restored, first);
    }
}

#[test]
fn full_domain_shards_cover_every_word_without_running_them_in_normal_ci() {
    let mut next = 0u64;
    for index in 0..257 {
        let domain = RawDecodeDomain::full_shard(index, 257).expect("full shard must be valid");
        assert_eq!(u64::from(domain.first), next);
        assert!(domain.count > 0);
        next += domain.count;
        assert_eq!(domain.word_at_last(), Some((next - 1) as u32));
    }
    assert_eq!(next, u64::from(u32::MAX) + 1);
    assert!(matches!(
        RawDecodeDomain::full_shard(2, 2),
        Err(RawDecodeError::Shard {
            index: 2,
            shards: 2
        })
    ));
}

#[test]
fn artifact_parser_refuses_version_drift_and_impossible_completion() {
    let source = scan_raw_decoder(
        RawDecoder::Spu,
        RawDecodeDomain::new(0, 9).expect("bounded interval"),
        3,
        2,
        None,
    )
    .expect("scan must finish");
    let mut json = serde_json::to_value(&source).expect("artifact serializes");
    json["schema_version"] = 2.into();
    assert!(matches!(
        RawDecodeArtifact::parse_json(&json.to_string()),
        Err(RawDecodeError::Version {
            found: 2,
            supported: 1
        })
    ));
    json["schema_version"] = 1.into();
    json["processed"] = 8.into();
    assert!(matches!(
        RawDecodeArtifact::parse_json(&json.to_string()),
        Err(RawDecodeError::InvalidArtifact)
    ));
    json["processed"] = 9.into();
    json["new_untracked_field"] = true.into();
    assert!(matches!(
        RawDecodeArtifact::parse_json(&json.to_string()),
        Err(RawDecodeError::Json(_))
    ));
}

#[test]
fn artifact_parser_rejects_lost_or_duplicate_panic_witnesses() {
    let source = scan_raw_decoder(
        RawDecoder::Ppu,
        RawDecodeDomain::new(0, 3).expect("bounded interval"),
        3,
        1,
        None,
    )
    .expect("scan must finish");
    let mut json = serde_json::to_value(source).expect("artifact serializes");
    json["accepted"] = 1.into();
    json["refused"] = 0.into();
    json["panics"] = 2.into();
    let sample = serde_json::json!({"raw": 0, "payload": {"kind": "non_string"}});
    json["panic_samples"] = serde_json::json!([sample.clone()]);
    assert!(matches!(
        RawDecodeArtifact::parse_json(&json.to_string()),
        Err(RawDecodeError::InvalidArtifact)
    ));
    json["panic_samples"] = serde_json::json!([sample.clone(), sample]);
    assert!(matches!(
        RawDecodeArtifact::parse_json(&json.to_string()),
        Err(RawDecodeError::InvalidArtifact)
    ));
}

#[test]
fn cancellation_and_invalid_inputs_have_explicit_outcomes() {
    let domain = RawDecodeDomain::new(u32::MAX - 11, 12).expect("last words fit");
    let cancelled =
        scan_raw_decoder(RawDecoder::Spu, domain, 5, 2, Some(7)).expect("bounded prefix must run");
    assert_eq!(cancelled.status, RawDecodeStatus::Cancelled);
    assert_eq!(cancelled.processed, 7);
    assert_eq!(cancelled.accepted + cancelled.refused + cancelled.panics, 7);
    assert!(!cancelled.is_clean());
    assert_eq!(cancelled.word_at(11), Some(u32::MAX));
    let zero = scan_raw_decoder(RawDecoder::Ppu, domain, 5, 2, Some(0))
        .expect("empty prefix remains named");
    assert_eq!(
        (zero.status, zero.processed),
        (RawDecodeStatus::Cancelled, 0)
    );
    assert!(matches!(
        RawDecodeDomain::new(u32::MAX, 2),
        Err(RawDecodeError::Domain {
            first: u32::MAX,
            count: 2
        })
    ));
    assert!(matches!(
        RawDecodeDomain::new(0, 0),
        Err(RawDecodeError::Domain { first: 0, count: 0 })
    ));
    assert!(matches!(
        scan_raw_decoder(RawDecoder::Ppu, domain, 0, 2, None),
        Err(RawDecodeError::Chunk { requested: 0 })
    ));
    assert!(matches!(
        scan_raw_decoder(RawDecoder::Ppu, domain, MAX_RAW_DECODE_CHUNK + 1, 2, None),
        Err(RawDecodeError::Chunk { .. })
    ));
    assert!(matches!(
        scan_raw_decoder(RawDecoder::Ppu, domain, 5, 0, None),
        Err(RawDecodeError::Sweep(
            cellgov_fuzz::FiniteSweepError::ZeroWorkers
        ))
    ));
    assert!(matches!(
        scan_raw_decoder(RawDecoder::Ppu, domain, 5, 2, Some(13)),
        Err(RawDecodeError::Cancellation {
            offset: 13,
            count: 12
        })
    ));
}
