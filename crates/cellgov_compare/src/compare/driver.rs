//! Field-by-field driver functions: `compare` for a single pair and
//! `compare_multi` for a single CellGov observation against multiple
//! oracle baselines.

use crate::compare::events::find_event_divergence;
use crate::compare::memory::find_memory_divergence;
use crate::compare::types::{Classification, CompareMode, CompareResult, MultiCompareResult};
use crate::observation::Observation;

/// Compare two observations under `mode`, returning the first differing field.
pub fn compare(expected: &Observation, actual: &Observation, mode: CompareMode) -> CompareResult {
    let outcome_mismatch = if expected.outcome != actual.outcome {
        Some((expected.outcome, actual.outcome))
    } else {
        None
    };

    let memory_divergence = match mode {
        CompareMode::Strict | CompareMode::Memory => {
            find_memory_divergence(&expected.memory_regions, &actual.memory_regions)
        }
        CompareMode::Events | CompareMode::Prefix => None,
    };

    let event_divergence = match mode {
        CompareMode::Strict | CompareMode::Events => {
            find_event_divergence(&expected.events, &actual.events, false)
        }
        CompareMode::Prefix => find_event_divergence(&expected.events, &actual.events, true),
        CompareMode::Memory => None,
    };

    let classification = if outcome_mismatch.is_none()
        && memory_divergence.is_none()
        && event_divergence.is_none()
    {
        Classification::Match
    } else {
        Classification::Divergence
    };

    CompareResult {
        classification,
        mode,
        outcome_mismatch,
        memory_divergence,
        event_divergence,
    }
}

/// Compare a CellGov observation against multiple baselines.
///
/// # Panics
///
/// Panics if `baselines` is empty.
pub fn compare_multi(
    baselines: &[Observation],
    cellgov: &Observation,
    mode: CompareMode,
) -> MultiCompareResult {
    assert!(!baselines.is_empty(), "at least one baseline required");

    for i in 1..baselines.len() {
        let oracle_cmp = compare(&baselines[0], &baselines[i], mode);
        if oracle_cmp.classification == Classification::Divergence {
            return MultiCompareResult {
                classification: Classification::UnsettledOracle,
                mode,
                oracle_divergence: Some(oracle_cmp),
                cellgov_result: None,
            };
        }
    }

    let result = compare(&baselines[0], cellgov, mode);
    let classification = result.classification;
    MultiCompareResult {
        classification,
        mode,
        oracle_divergence: None,
        cellgov_result: Some(result),
    }
}

#[cfg(test)]
#[path = "tests/driver_tests.rs"]
mod tests;
