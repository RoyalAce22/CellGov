//! TraceLevel discriminant locking, variant distinctness, and default selection.

use super::*;

#[test]
fn discriminants_are_locked() {
    assert_eq!(TraceLevel::Scheduling as u8, 0);
    assert_eq!(TraceLevel::Effects as u8, 1);
    assert_eq!(TraceLevel::Commits as u8, 2);
    assert_eq!(TraceLevel::Hashes as u8, 3);
}

#[test]
fn variants_are_distinct() {
    let all = [
        TraceLevel::Scheduling,
        TraceLevel::Effects,
        TraceLevel::Commits,
        TraceLevel::Hashes,
    ];
    let unique: std::collections::BTreeSet<u8> = all.iter().map(|l| *l as u8).collect();
    assert_eq!(unique.len(), all.len());
}

#[test]
fn default_is_scheduling() {
    assert_eq!(TraceLevel::default(), TraceLevel::Scheduling);
}

#[test]
fn equality_distinguishes() {
    assert_eq!(TraceLevel::Effects, TraceLevel::Effects);
    assert_ne!(TraceLevel::Effects, TraceLevel::Commits);
}
