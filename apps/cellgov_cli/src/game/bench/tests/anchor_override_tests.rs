//! The anchor check against a run whose boot override set differs from
//! the anchor's.

use cellgov_compare::BootOverrides;
use cellgov_time::Budget;

use super::super::test_fixtures::{
    anchor_fixture, observed_stderr, test_identity, TEST_CHECKPOINT,
};
use super::*;

fn disagreements(recorded: &BootSummary, ran: &RunIdentity) -> Vec<String> {
    anchor_disagreements(
        recorded,
        ran,
        TEST_CHECKPOINT,
        390099,
        Budget::new(256),
        "MaxSteps",
        &observed_stderr(73, 100),
    )
}

#[test]
fn a_run_under_an_override_disagrees_with_a_clean_anchor() {
    let mut ran = test_identity();
    ran.overrides.skip_module_start = true;
    let failures = disagreements(&anchor_fixture(73), &ran);
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("recorded (none)") && failures[0].contains("ran skip_module_start"),
        "got {failures:?}"
    );
}

#[test]
fn an_anchor_that_names_an_override_disagrees_with_a_clean_run() {
    let mut recorded = anchor_fixture(73);
    recorded.identity.overrides = BootOverrides {
        prx_base: Some(0x3000_0000),
        ..BootOverrides::default()
    };
    let failures = disagreements(&recorded, &test_identity());
    assert_eq!(failures.len(), 1, "got {failures:?}");
    assert!(
        failures[0].contains("recorded prx_base=0x30000000") && failures[0].contains("ran (none)"),
        "got {failures:?}"
    );
}
