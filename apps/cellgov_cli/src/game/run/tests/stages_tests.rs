use super::state_trace_mismatch;

#[test]
fn a_state_trace_path_with_the_capture_on_agrees() {
    assert!(state_trace_mismatch(Some("run.trace"), true).is_none());
}

#[test]
fn no_state_trace_and_no_capture_agrees() {
    assert!(state_trace_mismatch(None, false).is_none());
}

#[test]
fn a_state_trace_path_without_the_capture_is_refused() {
    let refusal = state_trace_mismatch(Some("run.trace"), false).expect("a named refusal");
    assert!(refusal.contains("run.trace"), "{refusal}");
    assert!(refusal.contains("state hash"), "{refusal}");
}

#[test]
fn the_capture_without_a_state_trace_path_is_refused() {
    let refusal = state_trace_mismatch(None, true).expect("a named refusal");
    assert!(refusal.contains("no save-state-trace path"), "{refusal}");
    assert!(refusal.contains("determinism-check budget"), "{refusal}");
}
