use super::*;

#[test]
fn iteration_ids_wrap_without_dropping_cases() {
    let config = FuzzConfig {
        first_iteration: u64::MAX - 1,
        iterations: 4,
        ..FuzzConfig::default()
    };

    assert_eq!(
        config.iterations().collect::<Vec<_>>(),
        [u64::MAX - 1, u64::MAX, 0, 1]
    );
}

#[test]
fn replay_version_mismatch_is_a_typed_refusal() {
    let coordinates = ReplayCoordinates {
        campaign_version: CAMPAIGN_VERSION + 1,
        seed: 7,
        case_index: u64::MAX,
    };

    assert_eq!(
        coordinates.validate(),
        Err(ReplayVersionError {
            found: CAMPAIGN_VERSION + 1,
            supported: CAMPAIGN_VERSION,
        })
    );
}

#[test]
fn zero_case_campaign_is_a_harness_failure() {
    let run = ppu::run_instructions(FuzzConfig {
        iterations: 0,
        ..FuzzConfig::default()
    });

    assert!(matches!(
        run.outcome,
        RunOutcome::HarnessFailure(FuzzError::Configuration(ConfigurationError::ZeroIterations))
    ));
}
