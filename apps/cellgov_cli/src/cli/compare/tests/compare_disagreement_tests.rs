//! A refusal to observe that only one run raised exits as a disagreement.

use cellgov_compare::{DeterminismError, ObserveDisagreement, ObserveError, RegionExtractError};

use super::scenario::determinism_exit_status;
use crate::cli::exit_codes;

fn ghost() -> ObserveError {
    ObserveError::Region(RegionExtractError::SpaceMissing {
        name: "ghost".to_string(),
        space: 1,
        present: vec![0],
    })
}

#[test]
fn a_refusal_only_one_run_raised_is_a_disagreement() {
    for d in [
        ObserveDisagreement::FirstOnly(ghost()),
        ObserveDisagreement::SecondOnly(ghost()),
        ObserveDisagreement::Both {
            first: ghost(),
            second: ObserveError::Region(RegionExtractError::Empty {
                name: "nothing".to_string(),
                addr: 0,
            }),
        },
    ] {
        let e = DeterminismError::ObserveDisagreement(Box::new(d));
        assert_eq!(determinism_exit_status(&e), exit_codes::DISAGREED, "{e}");
    }
}

#[test]
fn a_refusal_both_runs_raised_the_same_way_still_ran_and_failed() {
    let e = DeterminismError::Observe(ghost());
    assert_eq!(determinism_exit_status(&e), exit_codes::FAILED);
}
