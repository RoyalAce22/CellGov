//! The sample JSON documents the reference publishes.

use super::schema;
use crate::cli::store::read::model::STORE_FORMAT_VERSION;

/// Every fenced block in the rendered section, without its fence.
fn json_blocks() -> Vec<String> {
    let rendered = schema::render();
    rendered
        .split("```json\n")
        .skip(1)
        .map(|rest| {
            rest.split_once("\n```")
                .expect("every opened fence is closed")
                .0
                .to_string()
        })
        .collect()
}

#[test]
fn every_sample_document_parses_as_json() {
    let blocks = json_blocks();
    assert!(!blocks.is_empty(), "no sample document was rendered");
    for block in blocks {
        serde_json::from_str::<serde_json::Value>(&block)
            .unwrap_or_else(|e| panic!("sample document is not JSON ({e}):\n{block}"));
    }
}

#[test]
fn every_sample_document_carries_the_store_format_version() {
    let blocks = json_blocks();
    assert!(!blocks.is_empty(), "no sample document was rendered");
    for block in blocks {
        let doc: serde_json::Value = serde_json::from_str(&block).unwrap();
        assert_eq!(
            doc["format_version"], STORE_FORMAT_VERSION,
            "missing or wrong format_version:\n{block}"
        );
    }
}

#[test]
fn every_store_document_names_the_commands_that_emit_it() {
    let rendered = schema::render();
    for command in [
        "`status`",
        "`firmware list`",
        "`firmware show`",
        "`firmware verify`",
        "`title list`",
        "`title show`",
        "`title verify`",
    ] {
        assert!(rendered.contains(command), "{command} names no document");
    }
}

#[test]
fn an_absent_optional_field_is_left_out_of_the_sample() {
    let rendered = schema::render();
    assert!(
        rendered.contains("\"image_version\""),
        "no optional field the sample does set was serialized, so the absence \
         checks below prove nothing"
    );
    for absent in ["manifest_error", "\"reason\"", "omission"] {
        assert!(
            !rendered.contains(absent),
            "{absent} is a `skip_serializing_if` field and the sample does not set it"
        );
    }
}

#[test]
fn rendering_is_byte_identical_across_two_invocations() {
    assert_eq!(schema::render(), schema::render());
}
