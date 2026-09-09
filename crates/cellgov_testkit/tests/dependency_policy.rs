//! Convention guard: the dependency graph carries no error-handling
//! framework.
//!
//! Every error type in the workspace is a local `thiserror` type. A
//! crate that offers an opaque error, a report type, or a context
//! macro pulls the workspace toward one universal error, which the
//! per-crate contract rules out. The lock file is the whole graph, so
//! a transitive pull-in fails here the same as a direct one.

use std::fs;
use std::path::{Path, PathBuf};

/// Crates the graph may not contain, at any depth.
const DENIED: &[&str] = &[
    "anyhow",
    "eyre",
    "color-eyre",
    "snafu",
    "error-stack",
    "fehler",
    "failure",
];

/// The one error-derive crate the workspace uses; its presence proves
/// the lock parser recognises package names.
const EXPECTED: &str = "thiserror";

/// Floor on the packages parsed. The graph is well past this; a
/// collapse below it means the parser stopped recognising entries.
const MIN_PACKAGES: usize = 50;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

/// Package names in a `Cargo.lock`: the `name = "..."` line of each
/// `[[package]]` entry.
fn package_names(lock: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_package = false;
    for line in lock.lines() {
        let line = line.trim();
        if line == "[[package]]" {
            in_package = true;
            continue;
        }
        if line.starts_with('[') {
            in_package = false;
            continue;
        }
        if in_package {
            if let Some(rest) = line.strip_prefix("name = \"") {
                if let Some(name) = rest.strip_suffix('"') {
                    names.push(name.to_string());
                }
            }
        }
    }
    names
}

#[test]
fn the_lock_parser_reads_package_names_and_nothing_else() {
    let lock = "version = 4\n\n[[package]]\nname = \"a\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"b\"\nversion = \"2.0.0\"\ndependencies = [\n \"a\",\n]\n\n[metadata]\nname = \"not-a-package\"\n";
    assert_eq!(package_names(lock), ["a", "b"]);
}

#[test]
fn the_dependency_graph_carries_no_error_framework() {
    let lock = workspace_root().join("Cargo.lock");
    let names = package_names(
        &fs::read_to_string(&lock)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", lock.display())),
    );
    assert!(
        names.len() >= MIN_PACKAGES,
        "gate went vacuous: only {} package(s) parsed from Cargo.lock",
        names.len()
    );
    assert!(
        names.iter().any(|n| n == EXPECTED),
        "{EXPECTED} is missing from Cargo.lock, so the parser is not reading the graph"
    );
    let present: Vec<&String> = names
        .iter()
        .filter(|n| DENIED.contains(&n.as_str()))
        .collect();
    assert!(
        present.is_empty(),
        "an error-handling framework reached the dependency graph: {present:?}. Every error \
         type is a local thiserror type; remove the dependency that pulls it in"
    );
}
