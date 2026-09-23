use super::*;

use serde_json::json;

#[test]
fn represented_values_serialize_with_the_status_tag_first() {
    let field = ReferenceField::Value { value: 7u32 };
    let encoded = "{\"status\":\"value\",\"value\":7}";

    assert_eq!(serde_json::to_string(&field).unwrap(), encoded);
    assert_eq!(
        serde_json::from_str::<ReferenceField<u32>>(encoded).unwrap(),
        field
    );
}

#[test]
fn undefined_fields_serialize_their_reason() {
    let field: ReferenceField<u32> = ReferenceField::Undefined {
        reason: "reserved bits".to_owned(),
    };
    let encoded = "{\"status\":\"undefined\",\"reason\":\"reserved bits\"}";

    assert_eq!(serde_json::to_string(&field).unwrap(), encoded);
    assert_eq!(
        serde_json::from_str::<ReferenceField<u32>>(encoded).unwrap(),
        field
    );
}

#[test]
fn unsupported_fields_serialize_their_reason() {
    let field: ReferenceField<u32> = ReferenceField::Unsupported {
        reason: String::new(),
    };
    let encoded = "{\"status\":\"unsupported\",\"reason\":\"\"}";

    assert_eq!(serde_json::to_string(&field).unwrap(), encoded);
    assert_eq!(
        serde_json::from_str::<ReferenceField<u32>>(encoded).unwrap(),
        field
    );
}

#[test]
fn nested_optional_values_round_trip_through_null() {
    let field: ReferenceField<Option<u64>> = ReferenceField::Value { value: None };
    let encoded = "{\"status\":\"value\",\"value\":null}";

    assert_eq!(serde_json::to_string(&field).unwrap(), encoded);
    assert_eq!(
        serde_json::from_str::<ReferenceField<Option<u64>>>(encoded).unwrap(),
        field
    );
}

#[test]
fn reference_fields_refuse_unknown_fields() {
    let error = serde_json::from_value::<ReferenceField<u32>>(json!({
        "status": "value",
        "value": 1,
        "extra": 2,
    }))
    .unwrap_err();

    assert_eq!(error.to_string(), "unknown field `extra`, expected `value`");
}

#[test]
fn reference_fields_refuse_a_capitalized_status() {
    let error =
        serde_json::from_value::<ReferenceField<u32>>(json!({ "status": "Value", "value": 1 }))
            .unwrap_err();

    assert_eq!(
        error.to_string(),
        "unknown variant `Value`, expected one of `value`, `undefined`, `unsupported`"
    );
}

#[test]
fn reference_fields_require_the_status_tag() {
    let error = serde_json::from_value::<ReferenceField<u32>>(json!({ "value": 1 })).unwrap_err();

    assert_eq!(error.to_string(), "missing field `status`");
}

#[test]
fn a_represented_value_requires_its_value() {
    let error =
        serde_json::from_value::<ReferenceField<u32>>(json!({ "status": "value" })).unwrap_err();

    assert_eq!(error.to_string(), "missing field `value`");
}

#[test]
fn reasons_do_not_make_different_statuses_equal() {
    let undefined: ReferenceField<u32> = ReferenceField::Undefined {
        reason: "x".to_owned(),
    };
    let unsupported: ReferenceField<u32> = ReferenceField::Unsupported {
        reason: "x".to_owned(),
    };

    assert_ne!(undefined, unsupported);
}
