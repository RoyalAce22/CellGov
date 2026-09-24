//! The status `diff compare` exits with when its twice-run check fails.

use cellgov_compare::{DeterminismError, ObserveError, RegionExtractError};

use super::scenario::determinism_exit_status;
use crate::cli::exit_codes;

#[test]
fn a_field_that_differs_between_the_two_runs_is_a_disagreement() {
    for e in [
        DeterminismError::OutcomeMismatch,
        DeterminismError::MemoryMismatch,
        DeterminismError::EventMismatch,
        DeterminismError::HashMismatch,
    ] {
        assert_eq!(determinism_exit_status(&e), exit_codes::DISAGREED, "{e}");
    }
}

#[test]
fn a_run_that_produced_no_observation_ran_and_failed() {
    let e = DeterminismError::Observe(ObserveError::Region(RegionExtractError::Empty {
        name: "nothing".to_string(),
        addr: 0,
    }));
    assert_eq!(determinism_exit_status(&e), exit_codes::FAILED);
}
