use std::fs;
use std::path::Path;

use super::*;

/// A tracked artifact of the current schema, and the same record one
/// schema behind: version 2 carried no fingerprint component, which
/// reads as an absent component and leaves the version check to refuse it.
fn current_and_previous() -> (String, String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("regressions/ppu-raw-sthu-ra-zero-debug-invariant.json");
    let current = fs::read_to_string(path).expect("the tracked artifact reads");
    let previous = current
        .replacen("\"schema_version\": 3", "\"schema_version\": 2", 1)
        .replacen(",\n    \"component\": null", "", 1);
    assert!(previous.contains("\"schema_version\": 2"));
    assert!(!previous.contains("\"component\""));
    (current, previous)
}

#[test]
fn an_artifact_of_another_schema_is_refused_by_its_version() {
    let (current, previous) = current_and_previous();
    assert!(FuzzFindingArtifact::parse_json(&current).is_ok());
    assert!(matches!(
        FuzzFindingArtifact::parse_json(&previous),
        Err(ArtifactError::Version {
            found: 2,
            supported: FINDING_ARTIFACT_VERSION,
        })
    ));
}
