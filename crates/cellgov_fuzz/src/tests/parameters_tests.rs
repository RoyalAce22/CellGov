use super::*;

use serde_json::json;

#[test]
fn values_return_the_choices_in_construction_order() {
    let parameters = ParameterStream::new(vec![3, 1, 2]);

    assert_eq!(parameters.values(), [3, 1, 2]);
}

#[test]
fn an_empty_stream_has_no_values_and_refuses_index_zero() {
    let mut parameters = ParameterStream::new(Vec::new());

    assert!(parameters.values().is_empty());
    assert_eq!(
        parameters.mutate(0, 1),
        Err(GeneratorError::ParameterIndex {
            index: 0,
            length: 0,
        })
    );
}

#[test]
fn the_last_index_is_mutable_and_holds_the_maximum_value() {
    let mut parameters = ParameterStream::new(vec![0, 0, 0]);

    assert_eq!(parameters.mutate(2, u32::MAX), Ok(()));
    assert_eq!(parameters.values(), [0, 0, u32::MAX]);
}

#[test]
fn an_index_at_the_length_is_refused_and_leaves_the_stream_unchanged() {
    let mut parameters = ParameterStream::new(vec![4, 5]);

    assert_eq!(
        parameters.mutate(2, 9),
        Err(GeneratorError::ParameterIndex {
            index: 2,
            length: 2,
        })
    );
    assert_eq!(parameters.values(), [4, 5]);
}

#[test]
fn the_maximum_index_is_refused_with_the_stream_length() {
    let mut parameters = ParameterStream::new(vec![4, 5]);

    assert_eq!(
        parameters.mutate(usize::MAX, 9),
        Err(GeneratorError::ParameterIndex {
            index: usize::MAX,
            length: 2,
        })
    );
}

#[test]
fn mutating_the_same_index_twice_keeps_the_last_value() {
    let mut parameters = ParameterStream::new(vec![1]);

    assert_eq!(parameters.mutate(0, 2), Ok(()));
    assert_eq!(parameters.mutate(0, 3), Ok(()));
    assert_eq!(parameters.values(), [3]);
}

#[test]
fn streams_serialize_as_bare_arrays() {
    let parameters = ParameterStream::new(vec![1, u32::MAX]);

    assert_eq!(
        serde_json::to_string(&parameters).unwrap(),
        "[1,4294967295]"
    );
    assert_eq!(
        serde_json::from_value::<ParameterStream>(json!([1, u32::MAX])).unwrap(),
        parameters
    );
    assert_eq!(
        serde_json::from_value::<ParameterStream>(json!([])).unwrap(),
        ParameterStream::new(Vec::new())
    );
}

#[test]
fn streams_refuse_values_outside_the_choice_width() {
    let error =
        serde_json::from_value::<ParameterStream>(json!([u64::from(u32::MAX) + 1])).unwrap_err();

    assert_eq!(
        error.to_string(),
        "invalid value: integer `4294967296`, expected u32"
    );
}
