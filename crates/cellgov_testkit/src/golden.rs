//! Golden-trace assertions: pin the exact record sequence a curated scenario
//! produces.
//!
//! Replay assertions only compare two runs against each other; goldens pin
//! against a known baseline and catch structural drift no replay check
//! observes.

use cellgov_trace::{TraceReader, TraceRecord};

/// Decode `actual_bytes` and assert exact equality against `expected`.
///
/// # Panics
///
/// Panics with scenario name, divergent record index, and the two records
/// on mismatch.
pub fn assert_golden_trace(scenario: &str, actual_bytes: &[u8], expected: &[TraceRecord]) {
    let actual: Vec<TraceRecord> = TraceReader::new(actual_bytes)
        .map(|r| r.expect("golden trace decode failed"))
        .collect();

    if actual.len() != expected.len() {
        panic!(
            "golden trace mismatch for '{scenario}': \
             expected {exp} records, got {act}",
            exp = expected.len(),
            act = actual.len()
        );
    }

    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "golden trace mismatch for '{scenario}' at record {i}");
    }
}

/// Like [`assert_golden_trace`], but only checks the first `expected.len()`
/// records.
///
/// For scenarios whose tail is in flux but whose prefix is stable.
pub fn assert_golden_trace_prefix(
    scenario: &str,
    actual_bytes: &[u8],
    expected_prefix: &[TraceRecord],
) {
    let actual: Vec<TraceRecord> = TraceReader::new(actual_bytes)
        .map(|r| r.expect("golden trace decode failed"))
        .collect();

    if actual.len() < expected_prefix.len() {
        panic!(
            "golden trace prefix mismatch for '{scenario}': \
             expected at least {exp} records, got {act}",
            exp = expected_prefix.len(),
            act = actual.len()
        );
    }

    for (i, (a, e)) in actual.iter().zip(expected_prefix.iter()).enumerate() {
        assert_eq!(
            a, e,
            "golden trace prefix mismatch for '{scenario}' at record {i}"
        );
    }
}

#[cfg(test)]
#[path = "tests/golden_tests.rs"]
mod tests;
