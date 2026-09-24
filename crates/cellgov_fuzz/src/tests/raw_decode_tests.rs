use super::*;

use std::error::Error as _;

use crate::TargetPanicPayload;

const FULL_WORDS: u64 = 1 << 32;

fn domain(first: u32, count: u64) -> RawDecodeDomain {
    RawDecodeDomain { first, count }
}

#[allow(clippy::too_many_arguments)]
fn artifact(
    first: u32,
    count: u64,
    status: RawDecodeStatus,
    processed: u64,
    accepted: u64,
    refused: u64,
    panics: u64,
    samples: &[u32],
) -> RawDecodeArtifact {
    RawDecodeArtifact {
        schema_version: RAW_DECODE_SCHEMA_VERSION,
        decoder: RawDecoder::Ppu,
        domain: domain(first, count),
        status,
        processed,
        accepted,
        refused,
        panics,
        panic_samples: samples
            .iter()
            .map(|&raw| DecodePanic {
                raw,
                payload: TargetPanicPayload::NonString,
            })
            .collect(),
    }
}

fn reparse(artifact: &RawDecodeArtifact) -> Result<RawDecodeArtifact, RawDecodeError> {
    RawDecodeArtifact::parse_json(&serde_json::to_string(artifact).expect("serializes"))
}

fn accepted_by(decoder: RawDecoder, words: impl Iterator<Item = u32>) -> u64 {
    words
        .filter(|&raw| match decoder {
            RawDecoder::Ppu => cellgov_ppu::decode::decode(raw).is_ok(),
            RawDecoder::Spu => cellgov_spu::decode::decode(raw).is_ok(),
        })
        .count() as u64
}

#[test]
fn decoder_and_status_serialize_as_snake_case() {
    assert_eq!(
        serde_json::to_string(&RawDecoder::Ppu).expect("serializes"),
        "\"ppu\""
    );
    assert_eq!(
        serde_json::to_string(&RawDecoder::Spu).expect("serializes"),
        "\"spu\""
    );
    assert_eq!(
        serde_json::to_string(&RawDecodeStatus::Complete).expect("serializes"),
        "\"complete\""
    );
    assert_eq!(
        serde_json::to_string(&RawDecodeStatus::Cancelled).expect("serializes"),
        "\"cancelled\""
    );
    assert_eq!(
        serde_json::from_str::<RawDecoder>("\"spu\"").expect("snake case decoder"),
        RawDecoder::Spu
    );
    assert_eq!(
        serde_json::from_str::<RawDecodeStatus>("\"cancelled\"").expect("snake case status"),
        RawDecodeStatus::Cancelled
    );
    assert!(serde_json::from_str::<RawDecoder>("\"Ppu\"").is_err());
    assert!(serde_json::from_str::<RawDecodeStatus>("\"Complete\"").is_err());
}

#[test]
fn domain_new_accepts_the_last_word_and_the_full_space() {
    assert_eq!(
        RawDecodeDomain::new(u32::MAX, 1).expect("last word"),
        domain(u32::MAX, 1)
    );
    assert_eq!(
        RawDecodeDomain::new(0, FULL_WORDS).expect("full space"),
        domain(0, FULL_WORDS)
    );
    assert_eq!(
        RawDecodeDomain::new(u32::MAX - 1, 2).expect("last two words"),
        domain(u32::MAX - 1, 2)
    );
    assert!(matches!(
        RawDecodeDomain::new(1, FULL_WORDS),
        Err(RawDecodeError::Domain {
            first: 1,
            count: FULL_WORDS
        })
    ));
    assert!(matches!(
        RawDecodeDomain::new(0, FULL_WORDS + 1),
        Err(RawDecodeError::Domain { first: 0, .. })
    ));
    assert!(matches!(
        RawDecodeDomain::new(u32::MAX - 1, 3),
        Err(RawDecodeError::Domain { count: 3, .. })
    ));
    assert!(matches!(
        RawDecodeDomain::new(u32::MAX, 0),
        Err(RawDecodeError::Domain {
            first: u32::MAX,
            count: 0
        })
    ));
    assert!(matches!(
        RawDecodeDomain::new(7, u64::MAX),
        Err(RawDecodeError::Domain {
            first: 7,
            count: u64::MAX
        })
    ));
}

#[test]
fn word_at_last_reports_the_final_word_only_inside_the_space() {
    assert_eq!(domain(u32::MAX, 1).word_at_last(), Some(u32::MAX));
    assert_eq!(domain(0, FULL_WORDS).word_at_last(), Some(u32::MAX));
    assert_eq!(domain(0, 1).word_at_last(), Some(0));
    assert_eq!(domain(0x10, 0x10).word_at_last(), Some(0x1f));
    assert_eq!(domain(u32::MAX, 2).word_at_last(), None);
    assert_eq!(domain(5, 0).word_at_last(), None);
    assert_eq!(domain(0, FULL_WORDS + 1).word_at_last(), None);
    assert_eq!(domain(u32::MAX, u64::MAX).word_at_last(), None);
}

#[test]
fn full_shard_splits_the_space_with_the_remainder_on_the_lowest_shards() {
    assert_eq!(
        RawDecodeDomain::full_shard(0, 1).expect("one shard"),
        domain(0, FULL_WORDS)
    );
    let base = FULL_WORDS / 3;
    let base_words = u32::try_from(base).expect("fits");
    assert_eq!(
        RawDecodeDomain::full_shard(0, 3).expect("first third"),
        domain(0, base + 1)
    );
    assert_eq!(
        RawDecodeDomain::full_shard(1, 3).expect("second third"),
        domain(base_words + 1, base)
    );
    let last = RawDecodeDomain::full_shard(2, 3).expect("last third");
    assert_eq!(last, domain(2 * base_words + 1, base));
    assert_eq!(last.word_at_last(), Some(u32::MAX));
    assert_eq!(
        RawDecodeDomain::full_shard(0, u32::MAX).expect("first of the most shards"),
        domain(0, 2)
    );
    assert_eq!(
        RawDecodeDomain::full_shard(1, u32::MAX).expect("second of the most shards"),
        domain(2, 1)
    );
    assert_eq!(
        RawDecodeDomain::full_shard(u32::MAX - 1, u32::MAX).expect("last of the most shards"),
        domain(u32::MAX, 1)
    );
    assert!(matches!(
        RawDecodeDomain::full_shard(0, 0),
        Err(RawDecodeError::Shard {
            index: 0,
            shards: 0
        })
    ));
    assert!(matches!(
        RawDecodeDomain::full_shard(3, 3),
        Err(RawDecodeError::Shard {
            index: 3,
            shards: 3
        })
    ));
    assert!(matches!(
        RawDecodeDomain::full_shard(u32::MAX, u32::MAX),
        Err(RawDecodeError::Shard {
            index: u32::MAX,
            shards: u32::MAX
        })
    ));
}

#[test]
fn error_display_names_every_refusal() {
    let inner = serde_json::from_str::<RawDecodeDomain>("{").expect_err("truncated JSON");
    let inner_text = inner.to_string();
    let json = RawDecodeError::from(inner);
    assert_eq!(
        json.to_string(),
        format!("raw decoder artifact JSON failed: {inner_text}")
    );
    assert!(json.source().is_some());
    assert_eq!(
        RawDecodeError::Version {
            found: 2,
            supported: 1
        }
        .to_string(),
        "raw decoder schema version 2 is unsupported; expected 1"
    );
    assert_eq!(
        RawDecodeError::InvalidArtifact.to_string(),
        "raw decoder artifact has inconsistent counts or panic samples"
    );
    assert_eq!(
        RawDecodeError::Domain {
            first: 0xabcd,
            count: 0
        }
        .to_string(),
        "raw decoder domain starting at 0x0000abcd cannot hold 0 words"
    );
    assert_eq!(
        RawDecodeError::Shard {
            index: 2,
            shards: 2
        }
        .to_string(),
        "raw decoder shard 2 is invalid for 2 shards"
    );
    assert_eq!(
        RawDecodeError::Chunk { requested: 0 }.to_string(),
        "raw decoder chunk size 0 must be within 1..=65536"
    );
    assert_eq!(
        RawDecodeError::Cancellation {
            offset: 13,
            count: 12
        }
        .to_string(),
        "raw decoder cancellation offset 13 exceeds 12 words"
    );
    let sweep = RawDecodeError::Sweep(FiniteSweepError::ZeroWorkers);
    assert_eq!(
        sweep.to_string(),
        "raw decoder finite sweep failed: finite sweep requires at least one worker"
    );
    assert_eq!(
        sweep.source().map(ToString::to_string),
        Some("finite sweep requires at least one worker".to_owned())
    );
    assert!(RawDecodeError::InvalidArtifact.source().is_none());
}

#[test]
fn is_invalid_request_separates_caller_settings_from_artifact_and_execution_failures() {
    let invalid = [
        RawDecodeError::Domain { first: 0, count: 0 },
        RawDecodeError::Shard {
            index: 1,
            shards: 1,
        },
        RawDecodeError::Chunk { requested: 0 },
        RawDecodeError::Cancellation {
            offset: 2,
            count: 1,
        },
        RawDecodeError::Sweep(FiniteSweepError::ZeroWorkers),
    ];
    for error in &invalid {
        assert!(error.is_invalid_request(), "{error}");
    }
    let json = serde_json::from_str::<RawDecodeDomain>("{").expect_err("truncated JSON");
    let other = [
        RawDecodeError::Json(json),
        RawDecodeError::Version {
            found: 0,
            supported: 1,
        },
        RawDecodeError::InvalidArtifact,
        RawDecodeError::Sweep(FiniteSweepError::CounterOverflow),
        RawDecodeError::Sweep(FiniteSweepError::Cancelled {
            processed: 1,
            total: 2,
        }),
    ];
    for error in &other {
        assert!(!error.is_invalid_request(), "{error}");
    }
}

#[test]
fn artifact_round_trips_through_json_with_typed_panic_samples() {
    let mut source = artifact(
        0x100,
        4,
        RawDecodeStatus::Complete,
        4,
        1,
        1,
        2,
        &[0x100, 0x102],
    );
    source.decoder = RawDecoder::Spu;
    source.panic_samples[0].payload = TargetPanicPayload::StaticStr("static".to_owned());
    source.panic_samples[1].payload = TargetPanicPayload::String("owned".to_owned());
    let json = serde_json::to_string(&source).expect("serializes");
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert_eq!(value["decoder"], "spu");
    assert_eq!(value["status"], "complete");
    assert_eq!(value["domain"]["first"], 0x100);
    assert_eq!(value["panic_samples"][0]["payload"]["kind"], "static_str");
    assert_eq!(value["panic_samples"][1]["payload"]["message"], "owned");
    assert_eq!(
        RawDecodeArtifact::parse_json(&json).expect("round trip"),
        source
    );
}

#[test]
fn artifact_parser_refuses_unknown_and_missing_fields_at_every_level() {
    let source = artifact(0, 2, RawDecodeStatus::Complete, 2, 1, 0, 1, &[1]);
    let value = serde_json::to_value(&source).expect("serializes");
    let mut top = value.clone();
    top["extra"] = 1.into();
    assert!(matches!(
        RawDecodeArtifact::parse_json(&top.to_string()),
        Err(RawDecodeError::Json(_))
    ));
    let mut nested = value.clone();
    nested["domain"]["last"] = 1.into();
    assert!(matches!(
        RawDecodeArtifact::parse_json(&nested.to_string()),
        Err(RawDecodeError::Json(_))
    ));
    let mut sample = value.clone();
    sample["panic_samples"][0]["index"] = 0.into();
    assert!(matches!(
        RawDecodeArtifact::parse_json(&sample.to_string()),
        Err(RawDecodeError::Json(_))
    ));
    let mut missing = value;
    missing
        .as_object_mut()
        .expect("object")
        .remove("panics")
        .expect("field present");
    assert!(matches!(
        RawDecodeArtifact::parse_json(&missing.to_string()),
        Err(RawDecodeError::Json(_))
    ));
}

#[test]
fn artifact_parser_checks_version_then_domain_then_counts() {
    let mut source = artifact(0x20, 0, RawDecodeStatus::Complete, 0, 0, 0, 0, &[]);
    source.schema_version = RAW_DECODE_SCHEMA_VERSION + 1;
    assert!(matches!(
        reparse(&source),
        Err(RawDecodeError::Version {
            found: 2,
            supported: 1
        })
    ));
    source.schema_version = RAW_DECODE_SCHEMA_VERSION;
    assert!(matches!(
        reparse(&source),
        Err(RawDecodeError::Domain {
            first: 0x20,
            count: 0
        })
    ));
    let overflow = artifact(u32::MAX, 2, RawDecodeStatus::Complete, 2, 2, 0, 0, &[]);
    assert!(matches!(
        reparse(&overflow),
        Err(RawDecodeError::Domain {
            first: u32::MAX,
            count: 2
        })
    ));
    let last = artifact(u32::MAX, 1, RawDecodeStatus::Complete, 1, 0, 1, 0, &[]);
    assert_eq!(reparse(&last).expect("last word parses"), last);
}

#[test]
fn artifact_parser_requires_cancelled_runs_to_stop_before_the_end() {
    let complete = artifact(0, 3, RawDecodeStatus::Complete, 3, 3, 0, 0, &[]);
    assert_eq!(reparse(&complete).expect("complete"), complete);
    let short = artifact(0, 3, RawDecodeStatus::Complete, 2, 2, 0, 0, &[]);
    assert!(matches!(
        reparse(&short),
        Err(RawDecodeError::InvalidArtifact)
    ));
    let cancelled_at_end = artifact(0, 3, RawDecodeStatus::Cancelled, 3, 3, 0, 0, &[]);
    assert!(matches!(
        reparse(&cancelled_at_end),
        Err(RawDecodeError::InvalidArtifact)
    ));
    let cancelled = artifact(0, 3, RawDecodeStatus::Cancelled, 2, 1, 1, 0, &[]);
    assert_eq!(reparse(&cancelled).expect("cancelled prefix"), cancelled);
    let empty_prefix = artifact(0, 3, RawDecodeStatus::Cancelled, 0, 0, 0, 0, &[]);
    assert_eq!(reparse(&empty_prefix).expect("empty prefix"), empty_prefix);
}

#[test]
fn artifact_parser_refuses_count_sums_that_disagree_or_overflow() {
    let disagree = artifact(0, 4, RawDecodeStatus::Complete, 4, 2, 1, 0, &[]);
    assert!(matches!(
        reparse(&disagree),
        Err(RawDecodeError::InvalidArtifact)
    ));
    let overflow = artifact(0, 4, RawDecodeStatus::Complete, 4, u64::MAX, 1, 0, &[]);
    assert!(matches!(
        reparse(&overflow),
        Err(RawDecodeError::InvalidArtifact)
    ));
    let late_overflow = artifact(0, 4, RawDecodeStatus::Complete, 4, 0, u64::MAX, 1, &[]);
    assert!(matches!(
        reparse(&late_overflow),
        Err(RawDecodeError::InvalidArtifact)
    ));
}

#[test]
fn artifact_parser_bounds_panic_samples_by_the_retention_cap() {
    let cap = MAX_RAW_DECODE_PANIC_SAMPLES as u32;
    let capped = (0..cap).collect::<Vec<_>>();
    let retained = artifact(0, 256, RawDecodeStatus::Complete, 256, 56, 0, 200, &capped);
    assert_eq!(reparse(&retained).expect("capped samples"), retained);
    let over = (0..=cap).collect::<Vec<_>>();
    let too_many = artifact(0, 256, RawDecodeStatus::Complete, 256, 56, 0, 200, &over);
    assert!(matches!(
        reparse(&too_many),
        Err(RawDecodeError::InvalidArtifact)
    ));
    let under = (0..cap - 1).collect::<Vec<_>>();
    let too_few = artifact(0, 256, RawDecodeStatus::Complete, 256, 56, 0, 200, &under);
    assert!(matches!(
        reparse(&too_few),
        Err(RawDecodeError::InvalidArtifact)
    ));
    let exact = artifact(0, 4, RawDecodeStatus::Complete, 4, 1, 0, 3, &[0, 2, 3]);
    assert_eq!(reparse(&exact).expect("every panic sampled"), exact);
    let lost = artifact(0, 4, RawDecodeStatus::Complete, 4, 1, 0, 3, &[0, 2]);
    assert!(matches!(
        reparse(&lost),
        Err(RawDecodeError::InvalidArtifact)
    ));
}

#[test]
fn artifact_parser_refuses_samples_outside_the_processed_prefix_or_out_of_order() {
    let inside = artifact(10, 8, RawDecodeStatus::Cancelled, 5, 3, 0, 2, &[10, 14]);
    assert_eq!(reparse(&inside).expect("samples in prefix"), inside);
    for samples in [[9, 14], [10, 15], [14, 10], [10, 10]] {
        let wrong = artifact(10, 8, RawDecodeStatus::Cancelled, 5, 3, 0, 2, &samples);
        assert!(
            matches!(reparse(&wrong), Err(RawDecodeError::InvalidArtifact)),
            "{samples:?}"
        );
    }
}

#[test]
fn is_clean_requires_completion_without_panics_and_matching_counts() {
    let clean = artifact(0, 4, RawDecodeStatus::Complete, 4, 3, 1, 0, &[]);
    assert!(clean.is_clean());
    let cancelled = artifact(0, 4, RawDecodeStatus::Cancelled, 3, 2, 1, 0, &[]);
    assert!(!cancelled.is_clean());
    let panicked = artifact(0, 4, RawDecodeStatus::Complete, 4, 2, 1, 1, &[0]);
    assert!(!panicked.is_clean());
    let short = artifact(0, 4, RawDecodeStatus::Complete, 3, 2, 1, 0, &[]);
    assert!(!short.is_clean());
    let mismatched = artifact(0, 4, RawDecodeStatus::Complete, 4, 4, 1, 0, &[]);
    assert!(!mismatched.is_clean());
    let overflow = artifact(0, 4, RawDecodeStatus::Complete, 4, u64::MAX, 1, 0, &[]);
    assert!(!overflow.is_clean());
}

#[test]
fn word_at_maps_offsets_inside_the_domain_only() {
    let last_two = artifact(u32::MAX - 1, 2, RawDecodeStatus::Complete, 2, 2, 0, 0, &[]);
    assert_eq!(last_two.word_at(0), Some(u32::MAX - 1));
    assert_eq!(last_two.word_at(1), Some(u32::MAX));
    assert_eq!(last_two.word_at(2), None);
    assert_eq!(last_two.word_at(u64::MAX), None);
    let wrapped = artifact(u32::MAX, 2, RawDecodeStatus::Complete, 2, 2, 0, 0, &[]);
    assert_eq!(wrapped.word_at(0), Some(u32::MAX));
    assert_eq!(wrapped.word_at(1), None);
    let full = artifact(0, FULL_WORDS, RawDecodeStatus::Cancelled, 0, 0, 0, 0, &[]);
    assert_eq!(full.word_at(FULL_WORDS - 1), Some(u32::MAX));
    assert_eq!(full.word_at(FULL_WORDS), None);
    let empty = artifact(0, 0, RawDecodeStatus::Complete, 0, 0, 0, 0, &[]);
    assert_eq!(empty.word_at(0), None);
}

#[test]
fn scan_validates_domain_then_chunk_then_cancellation_then_workers() {
    let invalid = domain(0, 0);
    assert!(matches!(
        scan_raw_decoder(RawDecoder::Ppu, invalid, 0, 0, Some(5)),
        Err(RawDecodeError::Domain { first: 0, count: 0 })
    ));
    let valid = domain(0, 3);
    assert!(matches!(
        scan_raw_decoder(RawDecoder::Ppu, valid, 0, 0, Some(4)),
        Err(RawDecodeError::Chunk { requested: 0 })
    ));
    assert!(matches!(
        scan_raw_decoder(RawDecoder::Ppu, valid, 1, 0, Some(4)),
        Err(RawDecodeError::Cancellation {
            offset: 4,
            count: 3
        })
    ));
    assert!(matches!(
        scan_raw_decoder(RawDecoder::Ppu, valid, 1, 0, None),
        Err(RawDecodeError::Sweep(FiniteSweepError::ZeroWorkers))
    ));
    let widest = scan_raw_decoder(RawDecoder::Ppu, valid, MAX_RAW_DECODE_CHUNK, 1, None)
        .expect("largest chunk is allowed");
    assert_eq!(widest.processed, 3);
}

#[test]
fn scan_cancellation_at_the_domain_end_is_complete() {
    let selected = domain(100, 7);
    let full = scan_raw_decoder(RawDecoder::Spu, selected, 3, 2, None).expect("full scan");
    let at_end =
        scan_raw_decoder(RawDecoder::Spu, selected, 3, 2, Some(7)).expect("end boundary scan");
    assert_eq!(at_end, full);
    assert_eq!(at_end.status, RawDecodeStatus::Complete);
    assert_eq!(at_end.processed, 7);
    assert!(at_end.is_clean());
    let before_end =
        scan_raw_decoder(RawDecoder::Spu, selected, 3, 2, Some(6)).expect("prefix scan");
    assert_eq!(before_end.status, RawDecodeStatus::Cancelled);
    assert_eq!(before_end.processed, 6);
    assert_eq!(
        before_end.accepted + before_end.refused + before_end.panics,
        6
    );
    assert_eq!(
        reparse(&before_end).expect("cancelled artifact parses"),
        before_end
    );
}

#[test]
fn scan_chunking_and_worker_counts_do_not_change_the_artifact() {
    let selected = domain(0x3860_0000, 7);
    let single = scan_raw_decoder(RawDecoder::Ppu, selected, 1, 1, None).expect("chunk of one");
    let whole = scan_raw_decoder(RawDecoder::Ppu, selected, 7, 5, None).expect("one chunk");
    let uneven = scan_raw_decoder(RawDecoder::Ppu, selected, 4, 3, None).expect("uneven chunks");
    assert_eq!(single, whole);
    assert_eq!(single, uneven);
    assert_eq!(single.processed, 7);
    assert_eq!(single.decoder, RawDecoder::Ppu);
    assert_eq!(single.domain, selected);
}

#[test]
fn scan_uses_the_named_decoder() {
    let selected = domain(0, 64);
    let ppu_expected = accepted_by(RawDecoder::Ppu, 0..64);
    let spu_expected = accepted_by(RawDecoder::Spu, 0..64);
    assert_ne!(ppu_expected, spu_expected);
    let ppu = scan_raw_decoder(RawDecoder::Ppu, selected, 16, 2, None).expect("PPU scan");
    let spu = scan_raw_decoder(RawDecoder::Spu, selected, 16, 2, None).expect("SPU scan");
    assert_eq!((ppu.decoder, ppu.accepted), (RawDecoder::Ppu, ppu_expected));
    assert_eq!((spu.decoder, spu.accepted), (RawDecoder::Spu, spu_expected));
    assert_eq!(ppu.refused, 64 - ppu_expected);
    assert_eq!(spu.refused, 64 - spu_expected);
}

#[test]
fn scan_reaches_the_last_word_of_the_space() {
    let selected = domain(u32::MAX, 1);
    for decoder in [RawDecoder::Ppu, RawDecoder::Spu] {
        let scanned = scan_raw_decoder(decoder, selected, 1, 1, None).expect("last word scan");
        assert_eq!(scanned.processed, 1);
        assert_eq!(scanned.word_at(0), Some(u32::MAX));
        assert_eq!(
            scanned.accepted,
            accepted_by(decoder, std::iter::once(u32::MAX))
        );
        assert_eq!(scanned.accepted + scanned.refused, 1);
        assert!(scanned.is_clean());
    }
}

/// A host that stops the scan after its first batch, recording each
/// progress report.
#[derive(Default)]
struct StopAfterOne {
    scanned: Vec<u64>,
}

impl RawScanHost for StopAfterOne {
    fn expired(&self) -> bool {
        !self.scanned.is_empty()
    }

    fn scanned(&mut self, processed: u64) {
        self.scanned.push(processed);
    }
}

/// The host's deadline ends a scan between batches, and a scan that
/// stops short reports itself cancelled, not complete.
#[test]
fn a_host_that_stops_the_scan_leaves_it_cancelled_at_a_batch_boundary() {
    let domain = RawDecodeDomain::new(0, 300).expect("domain");
    let mut host = StopAfterOne::default();
    let artifact =
        scan_raw_decoder_with(RawDecoder::Ppu, domain, 128, 1, None, &mut host).expect("scans");
    assert_eq!(host.scanned, vec![128]);
    assert_eq!(artifact.processed, 128);
    assert_eq!(artifact.status, RawDecodeStatus::Cancelled);
    assert_eq!(artifact.accepted + artifact.refused, 128);

    let whole = scan_raw_decoder(RawDecoder::Ppu, domain, 128, 1, None).expect("scans");
    assert_eq!(whole.status, RawDecodeStatus::Complete);
    assert_eq!(whole.processed, 300);
}
