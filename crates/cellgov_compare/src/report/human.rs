//! Human-readable terminal renderers for single and multi-baseline reports.

use std::fmt::Write;

use crate::compare::{CompareResult, MultiCompareResult};

use super::labels::{classification_label, mode_str};

/// Format a comparison result as human-readable terminal text.
pub fn format_human(result: &CompareResult) -> String {
    let mut out = String::new();

    let class = classification_label(result.classification);
    let mode = mode_str(result.mode);

    let _ = writeln!(out, "classification: {class}");
    let _ = writeln!(out, "mode: {mode}");

    if let Some((expected, actual)) = &result.outcome_mismatch {
        let _ = writeln!(out, "outcome: expected {expected:?}, actual {actual:?}");
    }

    if let Some(d) = &result.memory_divergence {
        let _ = writeln!(
            out,
            "memory: region=\"{}\" offset={} expected=0x{:02x} actual=0x{:02x}",
            d.region, d.offset, d.expected, d.actual
        );
    }

    if let Some(d) = &result.event_divergence {
        let _ = write!(out, "events: index={}", d.index);
        match (&d.expected, &d.actual) {
            (Some(e), Some(a)) => {
                let _ = writeln!(
                    out,
                    " expected={:?}/unit={} actual={:?}/unit={}",
                    e.kind, e.unit, a.kind, a.unit
                );
            }
            (Some(e), None) => {
                let _ = writeln!(
                    out,
                    " expected={:?}/unit={} actual=<missing>",
                    e.kind, e.unit
                );
            }
            (None, Some(a)) => {
                let _ = writeln!(
                    out,
                    " expected=<missing> actual={:?}/unit={}",
                    a.kind, a.unit
                );
            }
            // compare::compare never constructs an EventDivergence with
            // both sides None: every divergence has at least one event.
            (None, None) => unreachable!("EventDivergence with both sides None"),
        }
    }

    out
}

/// Format a multi-baseline comparison result as human-readable terminal text.
pub fn format_multi_human(result: &MultiCompareResult, baseline_count: usize) -> String {
    let mut out = String::new();

    let class = classification_label(result.classification);

    let _ = writeln!(out, "classification: {class}");
    let _ = writeln!(out, "mode: {}", mode_str(result.mode));
    let _ = writeln!(out, "baselines: {baseline_count}");

    if let Some(ref div) = result.oracle_divergence {
        let _ = writeln!(out, "oracle: DISAGREE");
        let _ = write!(out, "{}", format_human(div));
    } else {
        let _ = writeln!(out, "oracle: AGREE");
        if let Some(ref cg) = result.cellgov_result {
            let _ = write!(out, "{}", format_human(cg));
        }
    }

    out
}

#[cfg(test)]
#[path = "tests/human_tests.rs"]
mod tests;
