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

/// Records each verdict as a line, in call order.
#[derive(Default)]
struct Log(Vec<String>);

impl ReferenceComparison for Log {
    type Component = u8;
    type Difference = &'static str;

    fn compared(&mut self, component: u8) {
        self.0.push(format!("compared {component}"));
    }

    fn differs(&mut self, component: u8, difference: &'static str) {
        self.0.push(format!("differs {component} {difference}"));
    }

    fn omitted(&mut self, component: u8, omission: ReferenceOmission, reason: &str) {
        self.0.push(format!(
            "omitted {component} {} {reason}",
            omission.as_str()
        ));
    }
}

#[test]
fn a_represented_field_is_compared_then_checked() {
    let mut log = Log::default();

    compare_field(
        &mut log,
        1,
        &ReferenceField::Value { value: 3u8 },
        |value| (*value != 4).then_some("3 vs 4"),
    );
    compare_field(
        &mut log,
        2,
        &ReferenceField::Value { value: 4u8 },
        |value| (*value != 4).then_some("4 vs 4"),
    );

    assert_eq!(log.0, ["compared 1", "differs 1 3 vs 4", "compared 2"]);
}

#[test]
fn an_omitted_field_keeps_its_status_and_reason_and_is_never_checked() {
    let mut log = Log::default();
    let undefined: ReferenceField<u8> = ReferenceField::Undefined {
        reason: "reserved bits".to_owned(),
    };
    let unsupported: ReferenceField<u8> = ReferenceField::Unsupported {
        reason: "not captured".to_owned(),
    };

    compare_field(&mut log, 5, &undefined, |_| {
        panic!("checked an undefined field")
    });
    compare_field(&mut log, 6, &unsupported, |_| {
        panic!("checked an unsupported field")
    });

    assert_eq!(
        log.0,
        [
            "omitted 5 undefined reserved bits",
            "omitted 6 unsupported not captured"
        ]
    );
}

#[test]
fn a_blank_reason_names_its_status_and_a_value_has_none() {
    let blank = |field: ReferenceField<u8>| field.blank_reason();

    assert_eq!(blank(ReferenceField::Value { value: 0 }), None);
    assert_eq!(
        blank(ReferenceField::Undefined {
            reason: " \t".to_owned()
        }),
        Some(ReferenceOmission::Undefined)
    );
    assert_eq!(
        blank(ReferenceField::Unsupported {
            reason: String::new()
        }),
        Some(ReferenceOmission::Unsupported)
    );
    assert_eq!(
        blank(ReferenceField::Undefined {
            reason: "x".to_owned()
        }),
        None
    );
}

fn capture(capture_id: &str, device: &str, environment: &str, digest: &str) -> ReferenceProvenance {
    ReferenceProvenance::HardwareCapture {
        capture_id: capture_id.to_owned(),
        device: device.to_owned(),
        environment: environment.to_owned(),
        source_sha256: digest.to_owned(),
    }
}

#[test]
fn a_documented_vector_reports_a_blank_id_before_its_citation() {
    let vector = |citation: &str, vector_id: &str| ReferenceProvenance::DocumentedVector {
        citation: citation.to_owned(),
        vector_id: vector_id.to_owned(),
    };

    assert_eq!(
        vector("bad", " ").check(|_| false),
        Err(ProvenanceFault::Blank("vector_id"))
    );
    assert_eq!(
        vector("bad", "v1").check(|citation| citation == "good"),
        Err(ProvenanceFault::Citation("bad"))
    );
    assert_eq!(
        vector("good", "v1").check(|citation| citation == "good"),
        Ok(())
    );
}

#[test]
fn a_hardware_capture_reports_its_first_failing_field() {
    let digest = "0123456789abcdef".repeat(4);

    assert_eq!(
        capture("", " ", "", "x").check(|_| true),
        Err(ProvenanceFault::Blank("capture_id"))
    );
    assert_eq!(
        capture("c", " ", "", "x").check(|_| true),
        Err(ProvenanceFault::Blank("device"))
    );
    assert_eq!(
        capture("c", "d", "", "x").check(|_| true),
        Err(ProvenanceFault::Blank("environment"))
    );
    assert_eq!(
        capture("c", "d", "e", &digest.to_uppercase()).check(|_| true),
        Err(ProvenanceFault::Digest)
    );
    assert_eq!(capture("c", "d", "e", &digest).check(|_| false), Ok(()));
}

#[test]
fn lower_hex_requires_the_exact_width_in_lowercase() {
    assert!(is_lower_hex("0a", 2));
    assert!(!is_lower_hex("0A", 2));
    assert!(!is_lower_hex("0a0", 2));
    assert!(!is_lower_hex("0g", 2));
    assert!(!is_lower_hex("", 2));
}
