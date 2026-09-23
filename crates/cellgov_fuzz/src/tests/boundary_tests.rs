use super::*;

#[test]
fn target_boundary_classifies_string_payloads() {
    let result = call_target(|| panic!("decoder failed"));

    assert_eq!(
        result,
        Err(TargetPanicPayload::StaticStr("decoder failed".to_owned()))
    );
}

#[test]
fn target_boundary_classifies_non_string_payloads_without_formatting_them() {
    let result = call_target(|| std::panic::panic_any(17_u32));

    assert_eq!(result, Err(TargetPanicPayload::NonString));
}

#[test]
fn panics_outside_target_calls_are_harness_failures() {
    for stage in ["generator", "comparator", "reducer", "worker", "sink"] {
        let result = call_harness(|| std::panic::panic_any(stage));

        assert_eq!(result, Err(HarnessPanic));
    }
}
