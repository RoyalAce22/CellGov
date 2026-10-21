//! Every hardware-capture SPU reference vector names a committed console
//! capture, `tests/micro/<probe>/ps3/<profile>/`, and carries that
//! capture's frame digest. A recapture changes `provenance.json`'s
//! `frame.sha256`; the vectors built from the old frame then fail here.

use std::collections::BTreeMap;
use std::path::Path;

use cellgov_fuzz::reference::ReferenceProvenance;
use cellgov_fuzz::spu_reference::{parse_reference_file, SpuReferenceFile};

/// Every hardware-capture provenance in `dir`, as (file, capture id,
/// digest).
fn capture_provenances(dir: &Path) -> Vec<(String, String, String)> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("the fixture directory reads")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    entries.sort();
    let mut found = Vec::new();
    for path in entries {
        let file = path
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();
        let text = std::fs::read_to_string(&path).expect("the fixture reads");
        let provenances =
            match parse_reference_file(&text).unwrap_or_else(|e| panic!("{file}: {e}")) {
                SpuReferenceFile::Single(artifact) => vec![artifact.provenance.clone()],
                SpuReferenceFile::Set(set) => {
                    set.vectors.iter().map(|v| v.provenance.clone()).collect()
                }
            };
        for provenance in provenances {
            if let ReferenceProvenance::HardwareCapture {
                capture_id,
                source_sha256,
                ..
            } = provenance
            {
                found.push((file.clone(), capture_id, source_sha256));
            }
        }
    }
    found
}

/// `frame.sha256` of the committed capture `capture_id` names.
fn committed_frame_digest(repo: &Path, capture_id: &str) -> String {
    let (probe, profile) = capture_id
        .split_once('/')
        .unwrap_or_else(|| panic!("{capture_id}: not <probe>/<profile>"));
    let path = repo
        .join("tests/micro")
        .join(probe)
        .join("ps3")
        .join(profile)
        .join("provenance.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{capture_id}: {}: {e}", path.display()));
    let provenance: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    provenance["frame"]["sha256"]
        .as_str()
        .unwrap_or_else(|| panic!("{}: no frame.sha256", path.display()))
        .to_owned()
}

#[test]
fn every_capture_vector_carries_its_committed_frame_digest() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo = crate_dir.join("../..");
    let found = capture_provenances(&crate_dir.join("tests/fixtures/spu_reference"));
    let mut frames = BTreeMap::new();
    for (file, capture_id, digest) in &found {
        let frame = frames
            .entry(capture_id.clone())
            .or_insert_with(|| committed_frame_digest(&repo, capture_id));
        assert_eq!(
            digest, frame,
            "{file}: {capture_id}: the vector's source_sha256 is not the committed frame's"
        );
    }
    assert!(
        found.len() >= 100,
        "checked {} capture vectors",
        found.len()
    );
    assert!(frames.len() >= 9, "checked {} captures", frames.len());
}
