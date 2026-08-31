//! The enum and the label table index each other; nothing but a test
//! keeps them aligned.

use super::*;

const ALL: [Phase; 7] = [
    Phase::Reading,
    Phase::Staging,
    Phase::Proving,
    Phase::Clearing,
    Phase::Committing,
    Phase::Hashing,
    Phase::ClearingStaging,
];

/// The label each variant must carry.
///
/// Exhaustive by construction: a variant added to [`Phase`] fails to
/// compile until it is answered here.
fn expected_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Reading => "reading",
        Phase::Staging => "staging",
        Phase::Proving => "verifying decrypt",
        Phase::Clearing => "clearing old install",
        Phase::Committing => "committing",
        Phase::Hashing => "hashing source",
        Phase::ClearingStaging => "clearing staging",
    }
}

#[test]
fn every_phase_indexes_its_own_label() {
    assert_eq!(INSTALL_TASK.phases.len(), ALL.len());
    for phase in ALL {
        assert_eq!(
            INSTALL_TASK.phases.get(phase.code() as usize).copied(),
            Some(expected_label(phase)),
            "{phase:?}"
        );
    }
}

#[test]
fn no_label_is_left_unclaimed_by_a_phase() {
    let claimed: std::collections::BTreeSet<usize> =
        ALL.iter().map(|p| p.code() as usize).collect();
    for (i, label) in INSTALL_TASK.phases.iter().enumerate() {
        assert!(
            claimed.contains(&i),
            "no phase claims phases[{i}] ({label})"
        );
    }
}

#[test]
fn the_measured_phase_is_the_one_with_a_byte_denominator() {
    assert_eq!(INSTALL_TASK.measured, Phase::Staging.code());
    assert_eq!(INSTALL_TASK.unit, Unit::Bytes);
    assert!(
        (INSTALL_TASK.measured as usize) < INSTALL_TASK.phases.len(),
        "the measured phase must name a label"
    );
}
