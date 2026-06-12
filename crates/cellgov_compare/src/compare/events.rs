//! Event-sequence diff (strict and prefix modes).

use crate::observation::ObservedEvent;

use super::types::EventDivergence;

/// In prefix mode, compares only up to the shorter sequence length.
pub(super) fn find_event_divergence(
    expected: &[ObservedEvent],
    actual: &[ObservedEvent],
    prefix: bool,
) -> Option<EventDivergence> {
    let compare_len = if prefix {
        expected.len().min(actual.len())
    } else {
        expected.len().max(actual.len())
    };

    for i in 0..compare_len {
        let e = expected.get(i);
        let a = actual.get(i);
        match (e, a) {
            (Some(e), Some(a)) => {
                if e.kind != a.kind || e.unit != a.unit {
                    return Some(EventDivergence {
                        index: i,
                        expected: Some(*e),
                        actual: Some(*a),
                    });
                }
            }
            (Some(e), None) => {
                return Some(EventDivergence {
                    index: i,
                    expected: Some(*e),
                    actual: None,
                });
            }
            (None, Some(a)) => {
                return Some(EventDivergence {
                    index: i,
                    expected: None,
                    actual: Some(*a),
                });
            }
            (None, None) => break,
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/events_tests.rs"]
mod tests;
