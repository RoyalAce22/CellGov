use super::buffer_reuse_tests::varied;
use super::*;
use crate::state::SpuObservableSnapshot;

/// A spare buffer that held another state's local store, one byte short.
fn dirty_buffer() -> Vec<u8> {
    let mut buffer = vec![0xc3; SPU_LS_SIZE];
    buffer.pop();
    buffer
}

#[test]
fn a_state_in_a_supplied_buffer_equals_a_new_one_and_keeps_the_buffer() {
    let buffer = dirty_buffer();
    let pointer = buffer.as_ptr();
    let state = SpuState::with_local_store(buffer);
    assert_eq!(state, SpuState::new());
    assert_eq!(state.ls.as_ptr(), pointer, "with_local_store reallocated");
}

#[test]
fn a_clone_into_a_supplied_buffer_equals_clone_and_keeps_the_buffer() {
    let source = varied();
    let buffer = dirty_buffer();
    let pointer = buffer.as_ptr();
    let copy = source.clone_with_local_store(buffer);
    assert_eq!(copy, source.clone());
    assert_eq!(
        copy.ls.as_ptr(),
        pointer,
        "clone_with_local_store reallocated"
    );
}

#[test]
fn a_taking_capture_equals_a_borrowed_one_and_empties_the_state() {
    let mut state = varied();
    let borrowed = SpuObservableSnapshot::capture(&state);
    let pointer = state.ls.as_ptr();
    let taken = SpuObservableSnapshot::capture_taking(&mut state);
    assert_eq!(taken, borrowed);
    assert_eq!(taken.ls.as_ptr(), pointer, "capture_taking copied");
    assert!(state.ls.is_empty());
}
