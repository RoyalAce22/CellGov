//! How a test run terminated.

use serde::{Deserialize, Serialize};

/// How a test run terminated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, thiserror::Error)]
pub enum ObservedOutcome {
    /// Title ran to a designated harness stop point: first RSX write,
    /// a manifest-declared PC checkpoint, or natural end-of-test in
    /// the synthetic-scenario runner.
    #[error("Completed")]
    Completed,
    /// Title called `sys_process_exit`. Distinct from `Completed`
    /// because a `sys_process_exit` may be an intentional title-side
    /// shutdown OR a synthesized exit from a fault path (for example,
    /// an unresolved import returning CELL_EINVAL which the title's
    /// CRT0 routes into `sys_process_exit`). Equality with
    /// `Completed` is intentionally false so cross-runner comparison
    /// surfaces this distinction.
    #[error("ProcessExit")]
    ProcessExit,
    /// No runnable units, but pending events or blocked receivers remain.
    #[error("Stalled")]
    Stalled,
    /// Max steps exceeded (CellGov) or wall-clock timeout (RPCS3).
    #[error("Timeout")]
    Timeout,
    /// Explicit runtime or architectural fault.
    #[error("Fault")]
    Fault,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_variants_are_distinct() {
        let outcomes = [
            ObservedOutcome::Completed,
            ObservedOutcome::ProcessExit,
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

    #[test]
    fn process_exit_does_not_equal_completed() {
        assert_ne!(ObservedOutcome::ProcessExit, ObservedOutcome::Completed);
    }
}
