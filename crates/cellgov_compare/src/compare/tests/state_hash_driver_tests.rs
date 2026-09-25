//! Same-runner state-hash comparison under every compare mode, in single
//! and multi-baseline form.

use super::*;
use crate::observation::{ObservedHashes, ObservedOutcome};
use crate::test_support::{meta, obs};
use cellgov_trace::StateHash;

fn hashes(memory: u64, unit_status: u64, sync: u64) -> ObservedHashes {
    ObservedHashes {
        memory: StateHash::new(memory),
        unit_status: StateHash::new(unit_status),
        sync: StateHash::new(sync),
        scheme: crate::observation::CHECKPOINT_HASH_SCHEME,
    }
}

fn hashed(runner: &str, h: ObservedHashes) -> Observation {
    let mut o = obs(ObservedOutcome::Completed, vec![], vec![]);
    o.metadata = meta(runner);
    o.state_hashes = Some(h);
    o
}

#[test]
fn a_same_runner_hash_mismatch_is_a_divergence_under_the_default_mode() {
    let baseline = hashed("cellgov", hashes(1, 2, 3));
    let fresh = hashed("cellgov", hashes(1, 2, 4));
    let r = compare(&baseline, &fresh, CompareMode::Strict);
    assert_eq!(r.classification, Classification::Divergence);
    assert_eq!(
        r.state_hash_divergence,
        Some(StateHashDivergence {
            expected: hashes(1, 2, 3),
            actual: hashes(1, 2, 4),
        })
    );
    assert!(r.outcome_mismatch.is_none());
    assert!(r.memory_divergence.is_none());
    assert!(r.event_divergence.is_none());
}

#[test]
fn equal_same_runner_hashes_match() {
    let a = hashed("cellgov", hashes(1, 2, 3));
    let b = a.clone();
    let r = compare(&a, &b, CompareMode::Strict);
    assert_eq!(r.classification, Classification::Match);
    assert!(r.state_hash_divergence.is_none());
}

#[test]
fn a_cross_runner_hash_mismatch_is_not_compared() {
    let a = hashed("cellgov", hashes(1, 2, 3));
    let b = hashed("other", hashes(9, 9, 9));
    let r = compare(&a, &b, CompareMode::Strict);
    assert_eq!(r.classification, Classification::Match);
    assert!(r.state_hash_divergence.is_none());
}

#[test]
fn a_side_without_hashes_is_not_compared() {
    let a = hashed("cellgov", hashes(1, 2, 3));
    let mut b = a.clone();
    b.state_hashes = None;
    for (x, y) in [(&a, &b), (&b, &a)] {
        let r = compare(x, y, CompareMode::Strict);
        assert_eq!(r.classification, Classification::Match);
        assert!(r.state_hash_divergence.is_none());
    }
}

#[test]
fn every_mode_compares_same_runner_state_hashes() {
    // The CLI's default mode is not strict; a stale baseline must fail
    // on the default path too.
    let a = hashed("cellgov", hashes(1, 2, 3));
    let b = hashed("cellgov", hashes(4, 5, 6));
    for mode in [
        CompareMode::Strict,
        CompareMode::Memory,
        CompareMode::Events,
        CompareMode::Prefix,
    ] {
        let r = compare(&a, &b, mode);
        assert_eq!(
            r.classification,
            Classification::Divergence,
            "mode: {mode:?}"
        );
        assert!(r.state_hash_divergence.is_some(), "mode: {mode:?}");
    }
}

#[test]
fn two_same_runner_baselines_with_differing_hashes_unsettle_the_oracle() {
    let stale = hashed("cellgov", hashes(1, 2, 3));
    let current = hashed("cellgov", hashes(1, 2, 4));
    let fresh = current.clone();
    let r = compare_multi(&[stale, current], &fresh, CompareMode::Strict);
    assert_eq!(r.classification, Classification::UnsettledOracle);
    assert!(r.cellgov_result.is_none());
    let oracle = r.oracle_divergence.unwrap();
    assert_eq!(
        oracle.state_hash_divergence,
        Some(StateHashDivergence {
            expected: hashes(1, 2, 3),
            actual: hashes(1, 2, 4),
        })
    );
}

#[test]
fn a_hashless_oracle_beside_a_hashed_cellgov_baseline_still_settles() {
    // RPCS3 baselines carry no hashes, so a CellGov baseline in the same
    // observations directory cannot unsettle them on hashes alone.
    let mut rpcs3 = hashed("rpcs3-interpreter", hashes(0, 0, 0));
    rpcs3.state_hashes = None;
    let saved = hashed("cellgov", hashes(1, 2, 3));
    let fresh = hashed("cellgov", hashes(1, 2, 3));
    let r = compare_multi(&[rpcs3, saved], &fresh, CompareMode::Strict);
    assert_eq!(r.classification, Classification::Match);
    assert!(r.oracle_divergence.is_none());
}

#[test]
fn multi_compare_carries_the_hash_divergence_through() {
    let baseline = hashed("cellgov", hashes(1, 2, 3));
    let fresh = hashed("cellgov", hashes(1, 2, 4));
    let r = compare_multi(&[baseline], &fresh, CompareMode::Strict);
    assert_eq!(r.classification, Classification::Divergence);
    assert!(r.cellgov_result.unwrap().state_hash_divergence.is_some());
}
