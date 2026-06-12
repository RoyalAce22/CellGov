//! Pairwise distinctness of ObservedOutcome variants.

use super::*;
use strum::VariantArray;

/// Trip-wire: iterates `Self::VARIANTS` so a new variant is
/// automatically covered.
#[test]
fn outcome_variants_are_distinct() {
    for (i, a) in ObservedOutcome::VARIANTS.iter().enumerate() {
        for (j, b) in ObservedOutcome::VARIANTS.iter().enumerate() {
            if i == j {
                assert_eq!(a, b);
            } else {
                assert_ne!(a, b);
            }
        }
    }
}

#[test]
fn process_exit_does_not_equal_completed() {
    assert_ne!(ObservedOutcome::ProcessExit, ObservedOutcome::Completed);
}
