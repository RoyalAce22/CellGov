//! Convention guard: the PS3 runner links no runtime.
//!
//! The runner sits beside a console and runs no guest code. It reads
//! and writes observation records, and those live in
//! `cellgov_observation`, below the runtime. This guard walks the
//! runner's normal (non-dev, non-build) path dependencies through every
//! workspace manifest they reach and fails when the closure names a
//! crate that carries the runtime, an interpreter or the test fixtures.
//!
//! `Cargo.lock` lists dev-dependencies beside normal ones, so the walk
//! reads each member's `[dependencies]` table instead.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Crates the runner's normal closure must not reach.
const FORBIDDEN: &[&str] = &[
    "cellgov_core",
    "cellgov_ppu",
    "cellgov_lv2",
    "cellgov_testkit",
];

/// Where the walk starts, relative to the workspace root.
const RUNNER: &str = "bridges/runner_ps3";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

/// The `name = { path = "..." }` entries of a manifest's `[dependencies]`
/// table, as `(name, path)`.
fn normal_path_dependencies(manifest: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_table = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_table = line == "[dependencies]";
            continue;
        }
        if !in_table || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let Some(after) = value.split_once("path").map(|(_, rest)| rest) else {
            continue;
        };
        let Some(quoted) = after.split('"').nth(1) else {
            continue;
        };
        out.push((name.trim().to_string(), quoted.to_string()));
    }
    out
}

/// Every crate the normal path-dependency closure of `start` reaches.
fn closure(root: &Path, start: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut pending = vec![root.join(start)];
    while let Some(dir) = pending.pop() {
        let manifest = fs::read_to_string(dir.join("Cargo.toml"))
            .unwrap_or_else(|e| panic!("{}: {e}", dir.join("Cargo.toml").display()));
        for (name, path) in normal_path_dependencies(&manifest) {
            if seen.insert(name) {
                pending.push(dir.join(path));
            }
        }
    }
    seen
}

#[test]
fn the_manifest_reader_takes_normal_path_dependencies_and_nothing_else() {
    let manifest = r#"
[package]
name = "x"

[dependencies]
a = { path = "../a" }
# b = { path = "../b" }
serde = "1"
c = { version = "1", path = "../c" }

[dev-dependencies]
d = { path = "../d" }

[build-dependencies]
e = { path = "../e" }
"#;
    assert_eq!(
        normal_path_dependencies(manifest),
        [
            ("a".to_string(), "../a".to_string()),
            ("c".to_string(), "../c".to_string())
        ]
    );
}

#[test]
fn the_runner_reaches_no_runtime_crate() {
    let reached = closure(&workspace_root(), RUNNER);
    assert!(
        reached.contains("cellgov_observation"),
        "the walk did not reach the schema crate, so it reads nothing: {reached:?}"
    );
    let forbidden: Vec<&&str> = FORBIDDEN
        .iter()
        .filter(|name| reached.contains(**name))
        .collect();
    assert!(
        forbidden.is_empty(),
        "{RUNNER} links {forbidden:?} through its normal dependencies {reached:?}; \
         take the record types from cellgov_observation instead"
    );
}
