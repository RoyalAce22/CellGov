use super::*;

use crate::seeded::{seed, SeededDefect};

const NOP: u32 = 0x6000_0000;
const MFLR_R0_WITH_RC: u32 = 0x7c08_02a7;
const SYNC: u32 = 0x7c00_04ac;
const MFSPR_R3_DSISR: u32 = (31 << 26) | (3 << 21) | (18 << 16) | (339 << 1);

fn domain(first: u32, count: u64) -> RawDecodeDomain {
    RawDecodeDomain { first, count }
}

fn reparse(artifact: &DecodeCensusArtifact) -> Result<DecodeCensusArtifact, DecodeCensusError> {
    DecodeCensusArtifact::parse_json(&serde_json::to_string(artifact).expect("serializes"))
}

#[test]
fn classify_separates_the_canonical_reserved_alias_gap_and_unknown_words() {
    assert_eq!(classify(NOP), WordClass::Canonical);
    assert_eq!(classify(MFLR_R0_WITH_RC), WordClass::ReservedBits);
    assert_eq!(classify(SYNC), WordClass::Alias);
    assert_eq!(classify(MFSPR_R3_DSISR), WordClass::ArmUnimplemented);
    assert_eq!(classify(0), WordClass::NotRecognized);
    assert_eq!(classify(1 << 26), WordClass::NotRecognized);
    // Every bit set is fnmadd. f31,f31,f31,f31 with Rc, a valid A-form word.
    assert_eq!(classify(u32::MAX), WordClass::Canonical);
}

#[test]
fn a_rejection_whose_mnemonic_the_gap_tables_do_not_carry_is_unlisted() {
    assert_eq!(
        gap_mnemonic(Locator::Spr {
            op_mnemonic: "mfspr",
            spr: 18
        }),
        Some("mfdsisr")
    );
    assert_eq!(
        gap_mnemonic(Locator::Spr {
            op_mnemonic: "mtspr",
            spr: 18
        }),
        Some("mtdsisr")
    );
    assert_eq!(
        gap_mnemonic(Locator::Spr {
            op_mnemonic: "mftb",
            spr: 18
        }),
        None
    );
    assert_eq!(
        gap_mnemonic(Locator::Spr {
            op_mnemonic: "mfmsr",
            spr: 18
        }),
        None
    );
    assert_eq!(
        gap_mnemonic(Locator::Opcode {
            primary: 31,
            xo: 339
        }),
        None
    );
}

#[test]
fn a_seeded_decoder_panic_is_a_panic_class_and_a_sampled_finding() {
    let _guard = seed(SeededDefect::DecoderPanic);
    assert_eq!(classify(NOP), WordClass::Panic);
    let artifact = census(domain(NOP, 3)).expect("census under a seeded panic");
    assert_eq!(artifact.classes.panics, 3);
    assert_eq!(artifact.findings(), 3);
    assert!(!artifact.is_clean());
    assert_eq!(
        artifact.samples,
        vec![
            CensusSample {
                raw: NOP,
                finding: CensusFinding::Panic
            },
            CensusSample {
                raw: NOP + 1,
                finding: CensusFinding::Panic
            },
            CensusSample {
                raw: NOP + 2,
                finding: CensusFinding::Panic
            },
        ]
    );
}

/// A seeded encoder that returns the canonical word with bit 0 flipped
/// breaks the round trip two ways: the nop's flipped word decodes to
/// another instruction, and mflr's flipped word decodes to the same
/// instruction but differs from the original outside its reserved bits.
#[test]
fn a_seeded_encoder_mismatch_is_a_round_trip_failure_and_a_sampled_finding() {
    let _guard = seed(SeededDefect::EncoderMismatch);
    assert_eq!(classify(NOP), WordClass::RoundTripFailure);
    assert_eq!(classify(MFLR_R0_WITH_RC & !1), WordClass::RoundTripFailure);
    // An alias spelling re-decodes its canonical word first, so it fails too.
    assert_eq!(classify(SYNC), WordClass::RoundTripFailure);
    let artifact = census(domain(NOP, 2)).expect("census under a seeded mismatch");
    assert_eq!(artifact.classes.round_trip_failures, 2);
    assert_eq!(artifact.findings(), 2);
    assert!(!artifact.is_clean());
    assert_eq!(
        artifact.samples,
        vec![
            CensusSample {
                raw: NOP,
                finding: CensusFinding::RoundTripFailure
            },
            CensusSample {
                raw: NOP + 1,
                finding: CensusFinding::RoundTripFailure
            },
        ]
    );
}

#[test]
fn extended_keys_follow_each_primary_own_opcode_width() {
    assert_eq!(extended_key(SYNC), Some((31, 598)));
    assert_eq!(extended_key((4 << 26) | 0x4c4), Some((4, 0x4c4)));
    assert_eq!(extended_key((19 << 26) | (150 << 1) | 1), Some((19, 150)));
    assert_eq!(extended_key((30 << 26) | (8 << 1)), Some((30, 8)));
    assert_eq!(extended_key((59 << 26) | (21 << 1)), Some((59, 21)));
    assert_eq!(extended_key((63 << 26) | (72 << 1)), Some((63, 72)));
    assert_eq!(extended_key(NOP), None);
    assert_eq!(extended_key(0), None);
}

#[test]
fn a_census_counts_every_word_once_and_agrees_with_classify() {
    let first = (31 << 26) | (5 << 21) | (6 << 16) | (7 << 11);
    let selected = domain(first, 2048);
    let artifact = census(selected).expect("census");
    assert_eq!(artifact.domain, selected);
    assert_eq!(artifact.classes.total(), Some(2048));
    assert_eq!(artifact.primaries.len(), 1);
    assert_eq!(artifact.primaries[0].primary, 31);
    assert_eq!(artifact.primaries[0].counts.total(), Some(2048));
    assert_eq!(artifact.extended.len(), 1024);
    for row in &artifact.extended {
        assert_eq!(row.primary, 31);
        assert_eq!(row.counts.total(), Some(2));
    }
    let mut expected = ClassCounts::default();
    for offset in 0..2048 {
        expected.count(classify(first + offset));
    }
    assert_eq!(artifact.classes, expected);
    assert!(expected.canonical > 0 && expected.reserved_bits > 0 && expected.alias > 0);
    assert!(expected.not_recognized > 0);
    assert!(artifact.is_clean(), "{:?}", artifact.samples);
    assert_eq!(reparse(&artifact).expect("round trip"), artifact);
}

#[test]
fn a_census_across_a_primary_boundary_keeps_one_row_per_primary() {
    let last_of_primary_zero = (1u32 << 26) - 4;
    let artifact = census(domain(last_of_primary_zero, 8)).expect("census");
    assert_eq!(
        artifact
            .primaries
            .iter()
            .map(|row| (row.primary, row.counts.total()))
            .collect::<Vec<_>>(),
        vec![(0, Some(4)), (1, Some(4))]
    );
    assert_eq!(artifact.primary_zero_decoded(), 0);
    assert!(artifact.extended.is_empty());
    assert!(artifact.is_clean());
}

#[test]
fn a_census_refuses_an_empty_or_overflowing_domain() {
    assert!(matches!(
        census(domain(5, 0)),
        Err(DecodeCensusError::Domain(RawDecodeError::Domain {
            first: 5,
            count: 0
        }))
    ));
    assert!(matches!(
        census(domain(u32::MAX, 2)),
        Err(DecodeCensusError::Domain(RawDecodeError::Domain {
            first: u32::MAX,
            count: 2
        }))
    ));
}

#[test]
fn merging_adjacent_parts_equals_one_census_over_their_union() {
    let first = (31 << 26) | (9 << 21) | (8 << 16);
    let whole = census(domain(first, 600)).expect("whole");
    let a = census(domain(first, 250)).expect("a");
    let b = census(domain(first + 250, 350)).expect("b");
    assert_eq!(merge(&[b.clone(), a.clone()]).expect("merged"), whole);
    assert_eq!(merge(std::slice::from_ref(&whole)).expect("single"), whole);
    let _guard = seed(SeededDefect::DecoderPanic);
    let whole = census(domain(first, 200)).expect("whole under panic");
    let a = census(domain(first, 100)).expect("a under panic");
    let b = census(domain(first + 100, 100)).expect("b under panic");
    assert_eq!(whole.samples.len(), MAX_DECODE_CENSUS_SAMPLES);
    assert_eq!(merge(&[a, b]).expect("merged under panic"), whole);
}

#[test]
fn merge_refuses_no_parts_a_gap_and_an_overlap() {
    assert!(matches!(merge(&[]), Err(DecodeCensusError::NoParts)));
    let a = census(domain(NOP, 4)).expect("a");
    let gap = census(domain(NOP + 5, 4)).expect("gap");
    assert!(matches!(
        merge(&[a.clone(), gap]),
        Err(DecodeCensusError::Discontiguous {
            expected,
            found
        }) if expected == u64::from(NOP) + 4 && found == NOP + 5
    ));
    let overlap = census(domain(NOP + 3, 4)).expect("overlap");
    assert!(matches!(
        merge(&[a, overlap]),
        Err(DecodeCensusError::Discontiguous { .. })
    ));
}

#[test]
fn the_parser_refuses_a_foreign_version_an_unknown_field_and_disagreeing_counts() {
    let artifact = census(domain(NOP, 4)).expect("census");
    let mut versioned = artifact.clone();
    versioned.schema_version = DECODE_CENSUS_SCHEMA_VERSION + 1;
    assert!(matches!(
        reparse(&versioned),
        Err(DecodeCensusError::Version {
            found: 2,
            supported: 1
        })
    ));
    let mut value = serde_json::to_value(&artifact).expect("serializes");
    value["extra"] = 1.into();
    assert!(matches!(
        DecodeCensusArtifact::parse_json(&value.to_string()),
        Err(DecodeCensusError::Json(_))
    ));
    let mut short = artifact.clone();
    short.classes.canonical -= 1;
    assert!(matches!(
        reparse(&short),
        Err(DecodeCensusError::InvalidArtifact)
    ));
    let mut unsampled = artifact.clone();
    unsampled.classes.round_trip_failures += 1;
    unsampled.classes.canonical -= 1;
    assert!(matches!(
        reparse(&unsampled),
        Err(DecodeCensusError::InvalidArtifact)
    ));
    let mut misfiled = artifact.clone();
    misfiled.extended.push(ExtendedCensus {
        primary: 31,
        xo: 0,
        counts: BucketCounts::default(),
    });
    assert!(matches!(
        reparse(&misfiled),
        Err(DecodeCensusError::InvalidArtifact)
    ));
    let mut stray = artifact;
    stray.samples.push(CensusSample {
        raw: NOP + 9,
        finding: CensusFinding::Panic,
    });
    assert!(matches!(
        reparse(&stray),
        Err(DecodeCensusError::InvalidArtifact)
    ));
}

#[test]
fn a_decoded_word_under_primary_zero_is_a_finding() {
    let mut artifact = census(domain(0, 4)).expect("census");
    assert!(artifact.is_clean());
    artifact.classes.not_recognized -= 1;
    artifact.classes.canonical += 1;
    artifact.primaries[0].counts.not_recognized -= 1;
    artifact.primaries[0].counts.decoded += 1;
    artifact.samples.push(CensusSample {
        raw: 0,
        finding: CensusFinding::PrimaryZeroDecoded,
    });
    assert_eq!(reparse(&artifact).expect("consistent artifact"), artifact);
    assert_eq!(artifact.primary_zero_decoded(), 1);
    assert_eq!(artifact.findings(), 1);
    assert!(!artifact.is_clean());
}

#[test]
fn the_parser_refuses_class_counts_that_disagree_with_the_primary_rows() {
    let artifact = census(domain(NOP, 4)).expect("census");
    // Every total and the finding count hold; only the split between
    // classes moves.
    let mut refiled = artifact.clone();
    refiled.classes.canonical -= 1;
    refiled.classes.arm_unimplemented += 1;
    assert!(matches!(
        reparse(&refiled),
        Err(DecodeCensusError::InvalidArtifact)
    ));
    let mut mismatched = artifact;
    mismatched.primaries[0].counts.decoded -= 1;
    mismatched.primaries[0].counts.panics += 1;
    assert!(matches!(
        reparse(&mismatched),
        Err(DecodeCensusError::InvalidArtifact)
    ));
}

#[test]
fn the_parser_refuses_extended_rows_that_do_not_tile_their_primary() {
    let first = (31 << 26) | (5 << 21) | (6 << 16) | (7 << 11);
    let artifact = census(domain(first, 4)).expect("census");
    assert_eq!(artifact.extended.len(), 2);
    assert_eq!(artifact.classes.panics, 0);
    let mut shifted = artifact.clone();
    let total = shifted.extended[0].counts.total().expect("fits");
    shifted.extended[0].counts = BucketCounts {
        panics: total,
        ..BucketCounts::default()
    };
    assert!(matches!(
        reparse(&shifted),
        Err(DecodeCensusError::InvalidArtifact)
    ));
    let mut wide = artifact;
    wide.extended.push(ExtendedCensus {
        primary: 31,
        xo: 0x400,
        counts: BucketCounts::default(),
    });
    assert!(matches!(
        reparse(&wide),
        Err(DecodeCensusError::InvalidArtifact)
    ));
}

#[test]
fn the_parser_refuses_counts_whose_finding_sum_overflows() {
    let mut artifact = census(domain(NOP, 4)).expect("census");
    artifact.classes.round_trip_failures = u64::MAX;
    artifact.classes.panics = 1;
    assert!(matches!(
        reparse(&artifact),
        Err(DecodeCensusError::InvalidArtifact)
    ));
}

#[test]
fn a_census_reaches_the_last_word_of_the_space() {
    let artifact = census(domain(u32::MAX - 1, 2)).expect("census");
    assert_eq!(artifact.classes.total(), Some(2));
    assert_eq!(
        artifact
            .primaries
            .iter()
            .map(|row| (row.primary, row.counts.total()))
            .collect::<Vec<_>>(),
        vec![(63, Some(2))]
    );
    assert_eq!(
        artifact
            .extended
            .iter()
            .map(|row| (row.xo, row.counts.total()))
            .collect::<Vec<_>>(),
        vec![(0x3FF, Some(2))]
    );
    assert_eq!(extended_key(u32::MAX), Some((63, 0x3FF)));
}

#[test]
fn error_display_names_every_refusal() {
    assert_eq!(
        DecodeCensusError::Version {
            found: 3,
            supported: 1
        }
        .to_string(),
        "decode census schema version 3 is unsupported; expected 1"
    );
    assert_eq!(
        DecodeCensusError::Discontiguous {
            expected: 0x10,
            found: 0x20
        }
        .to_string(),
        "decode census parts are not contiguous: expected a part starting at 0x00000010, found 0x00000020"
    );
    assert_eq!(
        DecodeCensusError::NoParts.to_string(),
        "decode census merge received no parts"
    );
    assert_eq!(
        DecodeCensusError::CounterOverflow.to_string(),
        "decode census counters overflowed"
    );
    assert_eq!(
        DecodeCensusError::InvalidArtifact.to_string(),
        "decode census artifact has inconsistent counts, histograms or samples"
    );
    let inner = RawDecodeError::Domain { first: 1, count: 0 };
    let inner_text = inner.to_string();
    assert_eq!(
        DecodeCensusError::Domain(inner).to_string(),
        format!("decode census domain is invalid: {inner_text}")
    );
}
