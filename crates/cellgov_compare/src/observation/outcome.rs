//! How a test run terminated.

use serde::{Deserialize, Serialize};

/// How a test run terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObservedOutcome {
    /// All units finished or explicitly blocked with no pending wakes.
    Completed,
    /// No runnable units, but pending events or blocked receivers remain.
    Stalled,
    /// Max steps exceeded (CellGov) or wall-clock timeout (RPCS3).
    Timeout,
    /// Explicit runtime or architectural fault.
    Fault,
}

impl std::fmt::Display for ObservedOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Completed => "Completed",
            Self::Stalled => "Stalled",
            Self::Timeout => "Timeout",
            Self::Fault => "Fault",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_variants_are_distinct() {
        let outcomes = [
            ObservedOutcome::Completed,
            ObservedOutcome::Stalled,
            ObservedOutcome::Timeout,
            ObservedOutcome::Fault,
        ];
        for (i, a) in outcomes.iter().enumerate() {
            for (j, b) in outcomes.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }
}
