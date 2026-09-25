//! Comparisons of state hashes under two schemes: each comparison names
//! the scheme mismatch, compares no hash, and claims no divergence.

use crate::observation::{ObservedHashes, ObservedOutcome};
use crate::test_support::{meta, obs, sample_observation};
use crate::{
    compare, compare_observations, diverge, format_human, format_observation_compare_human,
    trace_scheme, Classification, CompareMode, DivergeReport, Observation, StateHashCompare,
};
use cellgov_ppu::multilinear::SCHEME_ID;
use cellgov_ppu::state::FNV1A_SCHEME_ID;
use cellgov_trace::{StateHash, TraceRecord, TraceWriter, TRACE_FORMAT_VERSION};

fn header() -> TraceRecord {
    TraceRecord::RunIdentity {
        format_version: TRACE_FORMAT_VERSION,
        firmware: 1,
        game: 2,
        overrides: 0,
    }
}

/// A header, a scheme record for `Some(scheme)`, then three `PpuStateHash` records.
///
/// The three records do not depend on `scheme`, so two streams differ
/// only in their scheme record.
fn trace(scheme: Option<u64>) -> Vec<u8> {
    let mut w = TraceWriter::new();
    w.record_header(&header());
    if let Some(ppu) = scheme {
        w.record(&TraceRecord::StateHashScheme { ppu });
    }
    for step in 0..3 {
        w.record(&TraceRecord::PpuStateHash {
            step,
            pc: 0x100 + 4 * step,
            hash: StateHash::new(0xaa + step),
        });
    }
    w.take_bytes()
}

#[test]
fn a_stream_names_its_scheme_after_the_header() {
    assert_eq!(trace_scheme(&trace(Some(SCHEME_ID))), SCHEME_ID);
    assert_eq!(trace_scheme(&trace(Some(7))), 7);
}

#[test]
fn a_stream_without_a_scheme_record_reads_as_fnv1a() {
    assert_eq!(trace_scheme(&trace(None)), FNV1A_SCHEME_ID);
    assert_eq!(trace_scheme(&[]), FNV1A_SCHEME_ID);
}

#[test]
fn a_headerless_stream_may_lead_with_its_scheme() {
    let mut w = TraceWriter::new();
    w.record(&TraceRecord::StateHashScheme { ppu: 5 });
    assert_eq!(trace_scheme(&w.take_bytes()), 5);
}

#[test]
fn diverge_reports_a_scheme_mismatch_before_it_reads_a_record() {
    let old = trace(None);
    let new = trace(Some(SCHEME_ID));
    assert_eq!(
        diverge(&old, &new),
        DivergeReport::SchemeMismatch {
            a: FNV1A_SCHEME_ID,
            b: SCHEME_ID,
        }
    );
}

#[test]
fn a_stamped_fnv1a_stream_compares_with_an_unstamped_one() {
    let old = trace(None);
    let new = trace(Some(FNV1A_SCHEME_ID));
    assert_eq!(diverge(&old, &new), DivergeReport::Identical { count: 3 });
}

fn hashed(scheme: u64) -> Observation {
    let mut o = obs(ObservedOutcome::Completed, vec![], vec![]);
    o.metadata = meta("cellgov");
    o.state_hashes = Some(ObservedHashes {
        memory: StateHash::new(1),
        unit_status: StateHash::new(2),
        sync: StateHash::new(3),
        scheme,
    });
    o
}

#[test]
fn diff_observations_reports_a_scheme_mismatch_and_no_divergence() {
    let r = compare_observations(&hashed(FNV1A_SCHEME_ID), &hashed(SCHEME_ID));
    assert_eq!(
        r.state_hash_compare,
        StateHashCompare::SchemeMismatch {
            a: FNV1A_SCHEME_ID,
            b: SCHEME_ID,
        }
    );
    assert_eq!(r.scheme_mismatch(), Some((FNV1A_SCHEME_ID, SCHEME_ID)));
    assert!(!r.has_divergence());
    let human = format_observation_compare_human(&r);
    assert!(
        human.starts_with("SCHEME_MISMATCH state hashes: "),
        "{human}"
    );
    assert!(!human.contains("MATCH outcome"), "{human}");
}

#[test]
fn a_scheme_mismatch_leaves_other_divergences_standing() {
    let mut b = hashed(SCHEME_ID);
    b.outcome = ObservedOutcome::Fault;
    let r = compare_observations(&hashed(FNV1A_SCHEME_ID), &b);
    assert!(r.has_divergence());
    assert!(r.scheme_mismatch().is_some());
}

#[test]
fn a_cross_runner_pair_of_two_schemes_stays_a_note() {
    let mut b = hashed(SCHEME_ID);
    b.metadata = meta("rpcs3-interpreter");
    let r = compare_observations(&hashed(FNV1A_SCHEME_ID), &b);
    assert!(matches!(
        r.state_hash_compare,
        StateHashCompare::CrossRunnerNote { .. }
    ));
}

#[test]
fn diff_compare_classifies_a_scheme_mismatch() {
    for mode in [
        CompareMode::Strict,
        CompareMode::Memory,
        CompareMode::Events,
        CompareMode::Prefix,
    ] {
        let r = compare(&hashed(FNV1A_SCHEME_ID), &hashed(SCHEME_ID), mode);
        assert_eq!(r.classification, Classification::SchemeMismatch, "{mode:?}");
        assert_eq!(r.scheme_mismatch, Some((FNV1A_SCHEME_ID, SCHEME_ID)));
        assert!(r.state_hash_divergence.is_none());
        assert!(r.classification.exits_failure());
    }
    let r = compare(
        &hashed(FNV1A_SCHEME_ID),
        &hashed(SCHEME_ID),
        CompareMode::Strict,
    );
    assert!(
        format_human(&r).contains("classification: SCHEME_MISMATCH"),
        "{}",
        format_human(&r)
    );
}

#[test]
fn diff_compare_keeps_a_real_divergence_beside_a_scheme_mismatch() {
    let mut b = hashed(SCHEME_ID);
    b.outcome = ObservedOutcome::Fault;
    let r = compare(&hashed(FNV1A_SCHEME_ID), &b, CompareMode::Strict);
    assert_eq!(r.classification, Classification::Divergence);
}

#[test]
fn an_observation_without_a_scheme_reads_as_the_checkpoint_scheme() {
    let mut json: serde_json::Value = serde_json::to_value(sample_observation()).unwrap();
    let hashes = json["state_hashes"].as_object_mut().unwrap();
    assert!(hashes.remove("scheme").is_some());
    let back: Observation = serde_json::from_value(json).unwrap();
    assert_eq!(
        back.state_hashes.unwrap().scheme,
        crate::CHECKPOINT_HASH_SCHEME
    );
}

#[test]
fn a_changed_key_turns_a_comparison_into_a_scheme_mismatch() {
    use cellgov_ppu::multilinear::{scheme_id, KEYS, SCHEME_TAG};
    let mut keys = KEYS;
    keys[5] ^= 1;
    let moved = scheme_id(SCHEME_TAG, &keys);
    let a = trace(Some(SCHEME_ID));
    let b = trace(Some(moved));
    assert_eq!(
        diverge(&a, &b),
        DivergeReport::SchemeMismatch {
            a: SCHEME_ID,
            b: moved,
        }
    );
    assert_eq!(
        compare(&hashed(SCHEME_ID), &hashed(moved), CompareMode::Memory).classification,
        Classification::SchemeMismatch
    );
}

#[test]
fn a_corrupt_leading_record_is_a_corrupt_trace_not_a_scheme_mismatch() {
    let report = diverge(&[0xff], &trace(Some(SCHEME_ID)));
    assert!(
        matches!(
            report,
            DivergeReport::CorruptTrace {
                common_count: 0,
                a_error: Some(_),
                b_error: None,
            }
        ),
        "{report:?}"
    );
}

#[test]
fn two_baselines_of_two_schemes_settle_nothing() {
    let r = crate::compare_multi(
        &[hashed(FNV1A_SCHEME_ID), hashed(SCHEME_ID)],
        &hashed(FNV1A_SCHEME_ID),
        CompareMode::Strict,
    );
    assert_eq!(r.classification, Classification::UnsettledOracle);
    assert!(r.cellgov_result.is_none());
}

#[test]
fn the_checkpoint_scheme_is_neither_ppu_scheme() {
    assert_ne!(crate::CHECKPOINT_HASH_SCHEME, FNV1A_SCHEME_ID);
    assert_ne!(crate::CHECKPOINT_HASH_SCHEME, SCHEME_ID);
}
