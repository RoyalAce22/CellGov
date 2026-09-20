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
