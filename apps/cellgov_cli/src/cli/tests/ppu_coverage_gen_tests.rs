use std::path::Path;

use super::*;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/cellgov_cli has a workspace root two levels up")
}

const REGENERATE: &str =
    "regenerate with:\n  cargo run --release -p cellgov_cli -- dev ppu-coverage-gen";

#[test]
fn committed_ppu_coverage_doc_matches_generator() {
    let root = repo_root();
    let rendered = render(root, Path::new("docs/ppu_census.json")).expect("render the table");
    let path = root.join("docs/ppu_coverage.md");
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read {}: {e}; {REGENERATE}", path.display()));
    assert_eq!(
        committed.replace("\r\n", "\n"),
        rendered,
        "{} is stale; {REGENERATE}",
        path.display()
    );
}

#[test]
fn the_captured_probe_tables_are_read() {
    let words = captured_case_words(&repo_root().join(PROBE_DIR)).expect("read the probes");
    assert!(words.len() > 1000, "{} captured cases", words.len());
}
