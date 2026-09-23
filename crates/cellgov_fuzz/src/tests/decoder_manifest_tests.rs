use super::*;

use crate::raw_decode::{scan_raw_decoder, RawDecodeDomain, RawDecodeStatus};
use crate::semantic_sweep::{sweep_both, sweep_ppu};

fn sample_manifest() -> DecoderCampaignManifest {
    let (ppu, spu) = sweep_both();
    let first = scan_raw_decoder(
        RawDecoder::Ppu,
        RawDecodeDomain::new(0, 128).expect("domain"),
        23,
        2,
        None,
    )
    .expect("first");
    let second = scan_raw_decoder(
        RawDecoder::Ppu,
        RawDecodeDomain::new(128, 128).expect("domain"),
        31,
        3,
        None,
    )
    .expect("second");
    let spu_words = scan_raw_decoder(
        RawDecoder::Spu,
        RawDecodeDomain::new(0, 128).expect("domain"),
        17,
        1,
        None,
    )
    .expect("spu");
    let mut manifest = DecoderCampaignManifest::build(&[second, spu_words, first], &ppu, &spu)
        .expect("valid manifest");
    manifest
        .attach_reference(
            RawDecoder::Ppu,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/ppu_reference/li_r3_7_v1.json"
            )),
        )
        .expect("versioned documented PPU vector");
    manifest
        .attach_reference(
            RawDecoder::Spu,
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/spu_reference/rotqbyi_12_v1.json"
            )),
        )
        .expect("versioned documented SPU vector");
    manifest
}

#[test]
#[ignore = "regenerate the committed decoder coverage manifest intentionally"]
fn regenerate_committed_decoder_manifest() {
    let json = serde_json::to_string(&sample_manifest()).expect("manifest serializes");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/decoder_campaign_v1.json");
    std::fs::write(path, format!("{json}\n")).expect("writes fixture");
}

#[test]
fn committed_manifest_detects_witness_and_reserved_class_drift() {
    let baseline = DecoderCampaignManifest::parse_json(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/decoder_campaign_v1.json"
    )))
    .expect("committed versioned manifest");
    let current = sample_manifest();
    current
        .check_coverage(&baseline)
        .expect("decoder coverage unchanged");
    assert!(baseline
        .witnesses
        .iter()
        .any(|witness| witness.classes.contains(&SemanticCaseClass::ReservedField)));
    let mut removed_reserved = current;
    let reserved = removed_reserved
        .witnesses
        .iter_mut()
        .find(|witness| witness.classes.contains(&SemanticCaseClass::ReservedField))
        .expect("descriptor supplies reserved-field witness");
    reserved.classes = vec![SemanticCaseClass::Canonical];
    assert!(matches!(
        removed_reserved.check_coverage(&baseline),
        Err(DecoderManifestError::CoverageDrift)
    ));
}

#[test]
fn identical_campaign_inputs_serialize_identically_despite_partition_order() {
    let first = sample_manifest();
    let second = sample_manifest();
    let json = serde_json::to_string_pretty(&first).expect("serializes");
    assert_eq!(
        json,
        serde_json::to_string_pretty(&second).expect("serializes")
    );
    assert_eq!(
        DecoderCampaignManifest::parse_json(&json).expect("valid"),
        first
    );
    assert_eq!(first.raw_totals[0].words, 256);
    assert_eq!(first.raw_totals[1].words, 128);
    assert_eq!(
        first.raw_totals[0].accepted + first.raw_totals[0].refused,
        256
    );
    assert_eq!(
        first.raw_totals[1].accepted + first.raw_totals[1].refused,
        128
    );
    assert_eq!(first.authoritative_references.len(), 2);
    let source = crate::ppu_reference::parse_reference_json(
        &first.authoritative_references[0].artifact_json,
    )
    .expect("validated PPU source");
    assert_eq!(
        source.schema_version,
        crate::ppu_reference::PPU_REFERENCE_SCHEMA_VERSION
    );
    assert!(matches!(source.provenance,
        crate::ppu_reference::PpuReferenceProvenance::DocumentedVector {citation,vector_id}
        if citation == "PPC-Book1 p:51 s:3.3.8" && vector_id == "li-r3-7"));
}

#[test]
fn baseline_guard_rejects_a_lost_witness_and_a_changed_class() {
    let baseline = sample_manifest();
    let mut missing = baseline.clone();
    missing.witnesses.remove(0);
    assert!(matches!(
        missing.check_coverage(&baseline),
        Err(DecoderManifestError::CoverageDrift)
    ));
    let mut changed = baseline.clone();
    changed.witnesses[0].classes = vec![if changed.witnesses[0]
        .classes
        .contains(&SemanticCaseClass::Canonical)
    {
        SemanticCaseClass::OperandBoundary
    } else {
        SemanticCaseClass::Canonical
    }];
    assert!(matches!(
        changed.check_coverage(&baseline),
        Err(DecoderManifestError::CoverageDrift)
    ));
    assert!(baseline.check_coverage(&baseline).is_ok());
}

#[test]
fn swapped_semantic_reports_cannot_produce_wrong_decoder_replays() {
    let (ppu, spu) = sweep_both();
    assert!(matches!(
        DecoderCampaignManifest::build(&[], &spu, &ppu),
        Err(DecoderManifestError::SemanticCoverage)
    ));
}

#[test]
fn foreign_kind_in_a_semantic_finding_cannot_produce_wrong_replay() {
    let (mut ppu, spu) = sweep_both();
    let foreign = *spu.expected_kinds.iter().next().expect("SPU kind exists");
    ppu.findings
        .insert(SemanticSweepFinding::MissingKind { kind: foreign });
    assert!(matches!(
        DecoderCampaignManifest::build(&[], &ppu, &spu),
        Err(DecoderManifestError::SemanticCoverage)
    ));
}

#[test]
fn invalid_raw_partitions_and_corrupted_stored_totals_are_refused() {
    let baseline = sample_manifest();
    let (ppu, spu) = sweep_both();
    let mut empty = baseline.raw_partitions[2].clone();
    empty.domain.count = 0;
    empty.processed = 0;
    empty.accepted = 0;
    empty.refused = 0;
    assert!(matches!(
        DecoderCampaignManifest::build(&[empty], &ppu, &spu),
        Err(DecoderManifestError::RawPartitions)
    ));
    let mut gap = baseline.raw_partitions.clone();
    gap[1].domain.first += 1;
    assert!(matches!(
        DecoderCampaignManifest::build(&gap, &ppu, &spu),
        Err(DecoderManifestError::RawPartitions)
    ));

    let mut corrupted = baseline.clone();
    corrupted.raw_totals[0].accepted += 1;
    let json = serde_json::to_string(&corrupted).expect("serializes");
    assert!(matches!(
        DecoderCampaignManifest::parse_json(&json),
        Err(DecoderManifestError::RawPartitions)
    ));
}

#[test]
fn localization_preserves_original_and_fingerprint() {
    let fingerprint = DecoderFailureFingerprint {
        class: DecoderFailureClass::RawPanic,
        kind: None,
        other_kind: None,
        field: None,
        payload: Some(TargetPanicPayload::StaticStr("seeded".to_owned())),
        stage: None,
        case_class: None,
    };
    let original = DecoderReplay {
        decoder: RawDecoder::Ppu,
        raw: 0b1111_1001,
    };
    let minimized = minimize_decoder_failure(original, &fingerprint, |candidate| {
        (candidate.decoder == RawDecoder::Ppu && candidate.raw & 0b1001 == 0b1001)
            .then(|| fingerprint.clone())
    })
    .expect("same-class replay");
    assert_eq!(minimized.raw, 0b1001);
    assert_eq!(original.raw, 0b1111_1001);
    assert_eq!(
        minimize_decoder_failure(original, &fingerprint, |_| None),
        None
    );
    let mut finding = DecoderFailure {
        fingerprint: fingerprint.clone(),
        original: Some(original),
        descriptor_replay: None,
        minimized: None,
    };
    assert_eq!(
        finding
            .localize(|candidate| (candidate.raw & 0b1001 == 0b1001).then(|| fingerprint.clone())),
        Some(minimized)
    );
    assert_eq!(finding.original, Some(original));
    assert_eq!(finding.minimized, Some(minimized));
    assert_eq!(finding.localize(|_| None), None);
    assert_eq!(finding.minimized, None);
}

#[test]
fn kind_without_a_word_retains_exact_descriptor_replay() {
    let mut descriptors = cellgov_ppu::instruction::fuzz::generation_descriptors();
    let absent = descriptors.pop().expect("PPU descriptors exist");
    let (original_ppu, spu) = sweep_both();
    assert!(original_ppu.is_clean());
    let ppu = sweep_ppu(&descriptors);
    let manifest = DecoderCampaignManifest::build(&[], &ppu, &spu).expect("missing-kind manifest");
    let kind = format!("{:?}", crate::InstructionIdentity::Ppu(absent.kind));
    let missing = manifest
        .failures
        .iter()
        .find(|finding| {
            finding.fingerprint.class == DecoderFailureClass::MissingKind
                && finding.fingerprint.kind.as_deref() == Some(kind.as_str())
        })
        .expect("typed missing kind");
    assert_eq!(missing.original, None);
    assert_eq!(
        missing.descriptor_replay,
        Some(DecoderDescriptorReplay {
            decoder: RawDecoder::Ppu,
            kind
        })
    );
}

#[test]
fn typed_raw_and_semantic_failures_keep_original_replays_and_distinct_buckets() {
    let (mut ppu, spu) = sweep_both();
    let witness = ppu.witnesses.first().expect("PPU witness");
    let kind = witness.kind;
    let raw = witness.raw;
    ppu.findings.insert(SemanticSweepFinding::RoundTripFailure {
        kind,
        raw,
        encoded: None,
    });
    let artifact = RawDecodeArtifact {
        schema_version: RAW_DECODE_SCHEMA_VERSION,
        decoder: RawDecoder::Ppu,
        domain: RawDecodeDomain::new(raw, 3).expect("three words"),
        status: RawDecodeStatus::Complete,
        processed: 3,
        accepted: 0,
        refused: 0,
        panics: 3,
        panic_samples: (raw..raw + 3)
            .map(|raw| crate::DecodePanic {
                raw,
                payload: TargetPanicPayload::NonString,
            })
            .collect(),
    };
    let manifest = DecoderCampaignManifest::build(&[artifact], &ppu, &spu).expect("typed findings");
    assert_eq!(manifest.failures.len(), 4);
    assert_ne!(
        manifest.failures[0].fingerprint.class,
        manifest.failures[1].fingerprint.class
    );
    assert!(manifest.failures.iter().all(|finding| finding
        .original
        .is_some_and(|replay| replay.decoder == RawDecoder::Ppu)));
    assert!(manifest
        .failures
        .iter()
        .any(
            |finding| finding.fingerprint.class == DecoderFailureClass::RawPanic
                && finding.fingerprint.payload == Some(TargetPanicPayload::NonString)
        ));
    assert!(manifest
        .failures
        .iter()
        .any(
            |finding| finding.fingerprint.class == DecoderFailureClass::RoundTripFailure
                && finding.fingerprint.kind.as_deref() == Some(format!("{kind:?}").as_str())
        ));
}
