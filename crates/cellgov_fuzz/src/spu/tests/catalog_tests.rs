use std::path::{Path, PathBuf};

use cellgov_spu::fuzz::{sequence_relations, SpuFloatClass, SpuSequenceRelationId as Id};
use serde_json::Value;

use super::*;
use crate::spu::{check_fused_results, FusedResultVerdict};

fn docs_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

fn committed(file: &str) -> String {
    let path = docs_dir().join(file);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .replace("\r\n", "\n")
}

const REGENERATE: &str =
    "regenerate with:\n  cargo run --release -p cellgov_cli -- dev relations-gen";

#[test]
fn the_committed_catalog_matches_the_generator() {
    let catalog = relation_catalog().expect("the catalog renders");
    assert_eq!(
        committed(CATALOG_MARKDOWN),
        catalog.markdown,
        "docs/{CATALOG_MARKDOWN} is stale; {REGENERATE}"
    );
    assert_eq!(
        committed(CATALOG_JSON),
        catalog.json,
        "docs/{CATALOG_JSON} is stale; {REGENERATE}"
    );
}

#[test]
fn the_drift_gate_sees_a_row_added_without_a_regenerated_catalog() {
    let stored = stored_counterexamples().expect("the store parses");
    let mut rows = sequence_relations().to_vec();
    let current = render_catalog(&rows, &stored).expect("renders");
    rows.push(rows[0]);
    let added = render_catalog(&rows, &stored).expect("renders");
    assert_ne!(current.markdown, added.markdown);
    assert_ne!(current.json, added.json);
}

fn json_rows() -> Vec<Value> {
    let catalog: Value =
        serde_json::from_str(&relation_catalog().expect("renders").json).expect("the JSON parses");
    assert_eq!(catalog["schema_version"], CATALOG_SCHEMA_VERSION);
    catalog["rows"].as_array().expect("a row list").clone()
}

#[test]
fn the_catalog_lists_every_row_with_its_class_precondition_and_dead_set() {
    let rows = json_rows();
    let markdown = relation_catalog().expect("renders").markdown;
    assert_eq!(rows.len(), sequence_relations().len());
    for (row, entry) in sequence_relations().iter().zip(&rows) {
        let name = format!("{:?}", row.id);
        assert_eq!(entry["name"], name.as_str());
        assert!(markdown.contains(&format!("\n### {name}\n")), "{name}");
        let text = row.id.text();
        let (kind, ulp) = match row.float_class {
            SpuFloatClass::BitExact => ("BitExact", None),
            SpuFloatClass::BitExactUnderPrecondition => ("BitExactUnderPrecondition", None),
            SpuFloatClass::Inexact { ulp } => ("Inexact", ulp),
        };
        assert_eq!(entry["class"]["kind"], kind, "{name}");
        assert_eq!(entry["class"]["ulp"].as_u64(), ulp.map(u64::from), "{name}");
        assert_eq!(entry["precondition"].as_str(), text.precondition, "{name}");
        let dead: Vec<&str> = row
            .dead
            .iter()
            .map(|&register| text.registers[usize::from(register)])
            .collect();
        assert_eq!(entry["dead"], serde_json::json!(dead), "{name}");
        assert_eq!(
            entry["compares"]["claim"],
            if dead.is_empty() {
                "equality"
            } else {
                "refinement"
            },
            "{name}"
        );
        assert!(
            !entry["citations"].as_array().expect("a list").is_empty(),
            "{name}"
        );
    }
}

#[test]
fn each_stored_counterexample_is_listed_under_its_row_as_a_start_state() {
    let stored = stored_counterexamples().expect("the store parses");
    assert!(!stored.is_empty());
    let rows = json_rows();
    for counterexample in &stored {
        let entry = rows
            .iter()
            .find(|row| row["name"] == format!("{:?}", counterexample.relation).as_str())
            .expect("the row is listed");
        let listed = entry["counterexamples"]
            .as_array()
            .expect("a list")
            .iter()
            .find(|listed| listed["name"] == counterexample.name.as_str())
            .expect("the counterexample is listed");
        let parsed = RelationCounterexample::parse_json(&listed.to_string()).expect("it parses");
        assert_eq!(&parsed, counterexample);
    }
}

/// The ```json block of the page's result-file example.
fn documented_example() -> &'static str {
    let start = PREAMBLE
        .find("```json\n")
        .expect("the page shows an example")
        + 8;
    let end = start + PREAMBLE[start..].find("```").expect("the block closes");
    &PREAMBLE[start..end]
}

#[test]
fn the_documented_result_file_checks_as_a_match() {
    let results = check_fused_results(documented_example()).expect("the example is in form");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].relation, Id::CeqNotEqualFused);
    assert_eq!(results[0].verdict, FusedResultVerdict::Match);
}
