//! Every `BENCH_` line a boot emits is either a tracked witness or a
//! stated diagnostic, and the line table holds no row nothing emits.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Every `"BENCH_<NAME>:` string literal in `source`: the prefixes of
/// the stderr lines the boot path emits. A literal inside an
/// `#[error(...)]` attribute is an error's Display text, not a line.
fn emitted_bench_prefixes(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (start, _) in source.match_indices("\"BENCH_") {
        if source[..start].trim_end().ends_with("#[error(") {
            continue;
        }
        let body = &source[start + 1..];
        let name_len = body
            .bytes()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
            .count();
        if body[name_len..].starts_with(':') {
            out.insert(body[..=name_len].to_string());
        }
    }
    out
}

/// Every shipped `.rs` file at or below `dir`, in a deterministic
/// order.
///
/// The walk skips a `tests` directory: a fixture's `BENCH_` literal is
/// a line the test parses, so it names no emitter. Without the skip, a
/// line-table row outlives its last real emitter.
fn rust_sources_under(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let entries = std::fs::read_dir(&next)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", next.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if path.is_dir() {
                if name != "tests" {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && !name.ends_with("_tests.rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// The two trees that emit `BENCH_` lines -- the boot library, and the
/// bench driver that closes a measurement -- each with the name a
/// failure reports it by.
fn bench_line_emitters() -> [(&'static str, Vec<PathBuf>); 2] {
    let root = crate::paths::workspace_root();
    [
        (
            "the boot library",
            rust_sources_under(&root.join("crates").join("cellgov_boot").join("src")),
        ),
        (
            "the bench driver",
            rust_sources_under(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("src")
                    .join("game")
                    .join("bench"),
            ),
        ),
    ]
}

#[test]
fn every_emitted_bench_line_is_tracked_or_reasoned_diagnostic() {
    let mut emitted = BTreeSet::new();
    for (tree, paths) in bench_line_emitters() {
        let mut from_tree = BTreeSet::new();
        for path in paths {
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            from_tree.extend(emitted_bench_prefixes(&source));
        }
        // A tree the walk misses contributes nothing. The whole-scan
        // count below cannot tell that from one tree that carries every
        // line.
        assert!(
            !from_tree.is_empty(),
            "{tree} contributed no BENCH_ line; the walk is reading the wrong tree"
        );
        emitted.extend(from_tree);
    }
    assert!(
        emitted.len() > 20,
        "the scan found only {emitted:?}; the literal shape it keys on has moved"
    );

    let tracked: BTreeSet<String> = cellgov_compare::witness_parse::tracked_line_prefixes()
        .into_iter()
        .map(str::to_string)
        .collect();
    let diagnostic: BTreeSet<String> = cellgov_compare::witness_parse::diagnostic_lines()
        .iter()
        .map(|(p, _)| (*p).to_string())
        .collect();

    let unclassified: Vec<&String> = emitted
        .iter()
        .filter(|p| !tracked.contains(*p) && !diagnostic.contains(*p))
        .collect();
    assert!(
        unclassified.is_empty(),
        "emitted BENCH_ lines with no witness and no stated diagnostic-only reason: {unclassified:?}"
    );

    let stale: Vec<&String> = tracked
        .iter()
        .chain(diagnostic.iter())
        .filter(|p| !emitted.contains(*p))
        .collect();
    assert!(
        stale.is_empty(),
        "line-table rows no emitter produces: {stale:?}"
    );
}
