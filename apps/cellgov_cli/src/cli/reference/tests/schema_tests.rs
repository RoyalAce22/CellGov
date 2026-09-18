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
        "`firmware verify-corpus`",
        "`firmware kernels`",
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
    for absent in ["manifest_error", "omission"] {
        assert!(
            !rendered.contains(absent),
            "{absent} is a `skip_serializing_if` field and the sample does not set it"
        );
    }
}

#[test]
fn the_pup_corpus_sample_uses_reason_only_for_an_invalid_pup() {
    let rendered = schema::render();
    let block = rendered
        .split("`firmware verify-corpus`:\n\n```json\n")
        .nth(1)
        .and_then(|rest| rest.split_once("\n```"))
        .expect("the firmware verify-corpus sample is fenced JSON")
        .0;
    let doc: serde_json::Value = serde_json::from_str(block).expect("the sample is JSON");
    let mismatched = doc["mismatched"]
        .as_array()
        .expect("mismatched is an array");
    let invalid = mismatched
        .iter()
        .find(|row| row["kind"] == "invalid-pup")
        .expect("the sample includes an invalid PUP");
    assert!(invalid["reason"]
        .as_str()
        .is_some_and(|reason| !reason.is_empty()));
    let sha256 = mismatched
        .iter()
        .find(|row| row["kind"] == "sha256")
        .expect("the sample includes a SHA-256 mismatch");
    assert!(
        sha256.get("reason").is_none(),
        "a SHA-256 mismatch has no parse-failure reason"
    );
}

#[test]
fn the_pup_corpus_sample_does_not_reuse_one_hash_for_distinct_states() {
    let rendered = schema::render();
    let block = rendered
        .split("`firmware verify-corpus`:\n\n```json\n")
        .nth(1)
        .and_then(|rest| rest.split_once("\n```"))
        .expect("the firmware verify-corpus sample is fenced JSON")
        .0;
    let doc: serde_json::Value = serde_json::from_str(block).expect("the sample is JSON");

    let present = doc["present"][0]["pup_sha256"]
        .as_str()
        .expect("the present row names its hash");
    let missing = doc["missing"][0]["pup_sha256"]
        .as_str()
        .expect("the missing row names its hash");
    let expected = doc["mismatched"][0]["expected"][0]
        .as_str()
        .expect("the mismatch names an expected hash");
    let found = doc["mismatched"][0]["found"]
        .as_str()
        .expect("the mismatch names the found hash");

    assert_eq!(
        [present, missing, expected, found]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        4,
        "present, missing, and mismatched PUPs must describe distinct hashes"
    );
}

#[test]
fn the_kernel_coverage_sample_structures_non_decrypted_states() {
    let rendered = schema::render();
    let block = rendered
        .split("`firmware kernels`:\n\n```json\n")
        .nth(1)
        .and_then(|rest| rest.split_once("\n```"))
        .expect("the firmware kernels sample is fenced JSON")
        .0;
    let doc: serde_json::Value = serde_json::from_str(block).expect("the sample is JSON");
    let no_key = doc["entries"]
        .as_array()
        .expect("entries is an array")
        .iter()
        .find(|row| row["state"] == "no_key")
        .expect("the sample includes a no-key row");
    assert_eq!(no_key["kernel_version"], "1.50");
    let entries = doc["entries"].as_array().expect("entries is an array");
    let not_unpacked = entries
        .iter()
        .find(|row| row["state"] == "not_unpacked")
        .expect("the sample includes a not-unpacked row");
    assert_eq!(
        not_unpacked["detail"],
        "update_files carries no CORE_OS_PACKAGE.pkg"
    );
    assert!(entries.iter().any(|row| row["state"] == "not_installed"));
}

#[test]
fn rendering_is_byte_identical_across_two_invocations() {
    assert_eq!(schema::render(), schema::render());
}
