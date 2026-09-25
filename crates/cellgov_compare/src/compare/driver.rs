//! Field-by-field driver functions: `compare` for a single pair and
//! `compare_multi` for a single CellGov observation against multiple
//! oracle baselines.

use crate::compare::events::find_event_divergence;
use crate::compare::memory::find_memory_divergence;
use crate::compare::types::{
    Classification, CompareMode, CompareResult, MultiCompareResult, StateHashDivergence,
};
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

    let scheme_mismatch = find_scheme_mismatch(expected, actual);
    let state_hash_divergence = if scheme_mismatch.is_some() {
        None
    } else {
        find_state_hash_divergence(expected, actual)
    };

    let classification = if outcome_mismatch.is_some()
        || memory_divergence.is_some()
        || event_divergence.is_some()
        || state_hash_divergence.is_some()
    {
        Classification::Divergence
    } else if scheme_mismatch.is_some() {
        Classification::SchemeMismatch
    } else {
        Classification::Match
    };

    CompareResult {
        classification,
        mode,
        outcome_mismatch,
        memory_divergence,
        event_divergence,
        state_hash_divergence,
        scheme_mismatch,
    }
}

/// The (expected, actual) scheme ids when a same-runner pair holds state
/// hashes of two schemes.
fn find_scheme_mismatch(expected: &Observation, actual: &Observation) -> Option<(u64, u64)> {
    let (e, a) = (expected.state_hashes?, actual.state_hashes?);
    (expected.metadata.runner == actual.metadata.runner && e.scheme != a.scheme)
        .then_some((e.scheme, a.scheme))
}

/// Same-runner pairs only; see [`StateHashDivergence`].
fn find_state_hash_divergence(
    expected: &Observation,
    actual: &Observation,
) -> Option<StateHashDivergence> {
    let (e, a) = (expected.state_hashes?, actual.state_hashes?);
    if expected.metadata.runner != actual.metadata.runner || e == a {
        return None;
    }
    Some(StateHashDivergence {
        expected: e,
        actual: a,
    })
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
        // The driver compares no hash between two baselines of two
        // schemes, so the pair settles nothing.
        if matches!(
            oracle_cmp.classification,
            Classification::Divergence | Classification::SchemeMismatch
        ) {
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

#[cfg(test)]
#[path = "tests/state_hash_driver_tests.rs"]
mod state_hash_tests;
