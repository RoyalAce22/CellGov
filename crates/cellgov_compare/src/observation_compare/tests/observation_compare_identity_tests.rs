//! The identity triple carried through an observation comparison.

use super::*;
use crate::identity::RunIdentity;
use crate::observation::{Observation, ObservedOutcome};
use crate::test_support::{identity, obs};

fn with_identity(id: RunIdentity) -> Observation {
    let mut o = obs(ObservedOutcome::Completed, Vec::new(), Vec::new());
    o.identity = id;
    o
}

#[test]
fn the_result_carries_both_sides_triples() {
    let a = with_identity(identity("4.91", "NPAA00001", "base"));
    let b = with_identity(identity("4.93", "NPAA00001", "base"));
    let result = compare_observations(&a, &b);
    assert_eq!(result.a_identity, a.identity);
    assert_eq!(result.b_identity, b.identity);
}

#[test]
fn a_cross_triple_comparison_reports_the_warning() {
    let a = with_identity(identity("4.91", "NPAA00001", "base"));
    let b = with_identity(identity("4.93", "NPAA00001", "base"));
    let lines = compare_observations(&a, &b).identity_report("a.json", "b.json");
    assert!(
        lines.iter().any(|l| l.contains("cross-firmware")),
        "{lines:?}"
    );
}

#[test]
fn a_same_triple_comparison_reports_the_triple_without_a_warning() {
    let a = with_identity(identity("4.91", "NPAA00001", "base"));
    let lines = compare_observations(&a, &a).identity_report("a.json", "b.json");
    assert!(lines.iter().any(|l| l.contains("4.91")), "{lines:?}");
    assert!(!lines.iter().any(|l| l.contains("WARN")), "{lines:?}");
}

#[test]
fn a_cross_version_comparison_reports_the_warning() {
    let a = with_identity(identity("4.91", "NPAA00001", "base"));
    let b = with_identity(identity("4.91", "NPAA00001", "update:1.01"));
    let lines = compare_observations(&a, &b).identity_report("a.json", "b.json");
    assert!(
        lines.iter().any(|l| l.contains("cross-version")),
        "{lines:?}"
    );
}

/// Every cross-runner comparison takes this shape: only the CellGov
/// side composes from the store.
#[test]
fn an_unidentified_side_is_named_without_a_warning() {
    let a = with_identity(identity("4.91", "NPAA00001", "base"));
    let b = with_identity(RunIdentity::default());
    let lines = compare_observations(&a, &b).identity_report("cellgov.json", "oracle.json");
    assert!(
        lines.iter().any(|l| l.contains("(unidentified)")),
        "{lines:?}"
    );
    assert!(!lines.iter().any(|l| l.contains("WARN")), "{lines:?}");
}

#[test]
fn two_unidentified_observations_report_nothing() {
    let a = with_identity(RunIdentity::default());
    assert!(compare_observations(&a, &a)
        .identity_report("a.json", "b.json")
        .is_empty());
}

/// Two runs of different versions can still agree byte for byte.
#[test]
fn a_triple_mismatch_alone_is_not_a_divergence() {
    let a = with_identity(identity("4.91", "NPAA00001", "base"));
    let b = with_identity(identity("4.93", "NPAA00001", "base"));
    assert!(!compare_observations(&a, &b).has_divergence());
}
