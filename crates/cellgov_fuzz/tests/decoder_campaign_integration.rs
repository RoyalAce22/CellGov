//! Checks raw scans, semantic witnesses, replay, and independent references together.

use cellgov_fuzz::decoder_manifest::DecoderCampaignManifest;
use cellgov_fuzz::ppu_reference::{
    parse_reference_json as parse_ppu, replay_reference as replay_ppu,
};
use cellgov_fuzz::raw_decode::{scan_raw_decoder, RawDecodeDomain, RawDecoder};
use cellgov_fuzz::semantic_sweep::{sweep_both, SemanticCaseClass};
use cellgov_fuzz::spu_reference::{
    parse_reference_json as parse_spu, replay_reference as replay_spu,
};

const PPU_REFERENCE: &str = include_str!("fixtures/ppu_reference/li_r3_7_v1.json");
const SPU_REFERENCE: &str = include_str!("fixtures/spu_reference/rotqbyi_12_v1.json");
const BASELINE: &str = include_str!("fixtures/decoder_campaign_v1.json");

#[test]
fn raw_semantic_and_independent_reference_tiers_compose_without_changing_coverage() {
    let ppu_low = scan_raw_decoder(
        RawDecoder::Ppu,
        RawDecodeDomain::new(0, 128).expect("PPU low range"),
        16,
        4,
        None,
    )
    .expect("raw PPU low scan");
    let ppu_high = scan_raw_decoder(
        RawDecoder::Ppu,
        RawDecodeDomain::new(128, 128).expect("PPU high range"),
        19,
        2,
        None,
    )
    .expect("raw PPU high scan");
    let spu_low = scan_raw_decoder(
        RawDecoder::Spu,
        RawDecodeDomain::new(0, 128).expect("SPU low range"),
        11,
        3,
        None,
    )
    .expect("raw SPU low scan");
    let (ppu, spu) = sweep_both();
    assert!(ppu.is_clean() && spu.is_clean());

    let mut manifest = DecoderCampaignManifest::build(&[spu_low, ppu_high, ppu_low], &ppu, &spu)
        .expect("seam builds regardless of shard order");
    manifest
        .attach_reference(RawDecoder::Ppu, PPU_REFERENCE)
        .expect("validated PPU source");
    manifest
        .attach_reference(RawDecoder::Spu, SPU_REFERENCE)
        .expect("validated SPU source");
    let baseline = DecoderCampaignManifest::parse_json(BASELINE).expect("committed schema");
    manifest
        .check_coverage(&baseline)
        .expect("partitions and witnesses match committed baseline");
    assert!(manifest
        .witnesses
        .iter()
        .any(|witness| witness.classes.contains(&SemanticCaseClass::ReservedField)));
    assert!(manifest.failures.is_empty());
    assert_eq!(manifest.authoritative_references.len(), 2);

    let ppu_source = parse_ppu(PPU_REFERENCE).expect("PPU source parses");
    let ppu_observation = replay_ppu(&ppu_source).expect("PPU source replays");
    assert!(ppu_observation.internal_divergence.is_none());
    assert!(!ppu_observation.comparisons.is_empty());
    assert!(ppu_observation
        .comparisons
        .iter()
        .all(|comparison| comparison.is_match()));

    let spu_source = parse_spu(SPU_REFERENCE).expect("SPU source parses");
    let spu_observation = replay_spu(&spu_source).expect("SPU source replays");
    assert!(spu_observation.comparison.is_match());
    assert!(!spu_observation.comparison.compared.is_empty());
}
