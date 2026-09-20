//! `cellgov dev workspace-gen` -- Cargo-derived architecture-map regions.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;

use super::exit::die;
use super::parse::WorkspaceGenArgs;

const DEFAULT_OUTPUT: &str = "docs/architecture/workspace.md";
const DAG_START: &str = "<!-- workspace-gen:dag:start -->";
const DAG_END: &str = "<!-- workspace-gen:dag:end -->";
const EXTERNAL_START: &str = "<!-- workspace-gen:external:start -->";
const EXTERNAL_END: &str = "<!-- workspace-gen:external:end -->";

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    dependencies: Vec<Dependency>,
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    source: Option<String>,
    path: Option<PathBuf>,
    kind: Option<String>,
    target: Option<String>,
}

pub(crate) fn run(args: &WorkspaceGenArgs) {
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT));
    let metadata = cargo_metadata();
    let document = std::fs::read_to_string(&output)
        .unwrap_or_else(|e| die(&format!("workspace-gen: read {}: {e}", output.display())));
    let rendered = render_document(&document, &metadata)
        .unwrap_or_else(|e| die(&format!("workspace-gen: {}", e)));
    std::fs::write(&output, rendered)
        .unwrap_or_else(|e| die(&format!("workspace-gen: write {}: {e}", output.display())));
    println!("workspace-gen: wrote {}", output.display());
}

fn cargo_metadata() -> Metadata {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .unwrap_or_else(|e| die(&format!("workspace-gen: run cargo metadata: {e}")));
    if !output.status.success() {
        die(&format!(
            "workspace-gen: cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|e| die(&format!("workspace-gen: parse cargo metadata: {e}")))
}

fn render_document(document: &str, metadata: &Metadata) -> Result<String, String> {
    let dag = render_dag(metadata);
    let external = render_external_dependencies(metadata);
    let document = replace_region(document, DAG_START, DAG_END, &dag)?;
    replace_region(&document, EXTERNAL_START, EXTERNAL_END, &external)
}

fn replace_region(document: &str, start: &str, end: &str, body: &str) -> Result<String, String> {
    if document.matches(start).count() != 1 {
        return Err(format!("expected one {start}"));
    }
    if document.matches(end).count() != 1 {
        return Err(format!("expected one {end}"));
    }
    let start_at = document
        .find(start)
        .ok_or_else(|| format!("missing {start}"))?;
    let content_at = start_at + start.len();
    let end_at = document[content_at..]
        .find(end)
        .map(|offset| content_at + offset)
        .ok_or_else(|| format!("missing {end}"))?;
    Ok(format!(
        "{}\n{}\n{}{}",
        &document[..content_at],
        body,
        end,
        &document[end_at + end.len()..]
    ))
}

fn normal_dependencies(package: &Package) -> impl Iterator<Item = &Dependency> {
    package
        .dependencies
        .iter()
        .filter(|dependency| dependency.kind.is_none() && dependency.target.is_none())
}

fn render_dag(metadata: &Metadata) -> String {
    let names: BTreeSet<_> = metadata
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    let mut edges = BTreeSet::new();
    for package in &metadata.packages {
        for dependency in normal_dependencies(package) {
            if dependency.path.is_some() && names.contains(dependency.name.as_str()) {
                edges.insert((package.name.as_str(), dependency.name.as_str()));
            }
        }
    }

    let mut out = String::from("```mermaid\ngraph BT\n");
    for (index, name) in names.iter().enumerate() {
        out.push_str(&format!("  n{index}[\"{name}\"]\n"));
    }
    for (from, to) in edges {
        let from_index = names
            .iter()
            .position(|name| *name == from)
            .expect("workspace package");
        let to_index = names
            .iter()
            .position(|name| *name == to)
            .expect("workspace package");
        out.push_str(&format!("  n{to_index} --> n{from_index}\n"));
    }
    out.push_str("```\n");
    out
}

fn render_external_dependencies(metadata: &Metadata) -> String {
    let mut out = String::from("| Crate | Direct external dependencies |\n| --- | --- |\n");
    for package in &metadata.packages {
        let dependencies: BTreeSet<_> = normal_dependencies(package)
            .filter(|dependency| dependency.source.is_some())
            .map(|dependency| dependency.name.as_str())
            .collect();
        let dependencies = if dependencies.is_empty() {
            "none".to_string()
        } else {
            dependencies.into_iter().collect::<Vec<_>>().join(", ")
        };
        out.push_str(&format!("| `{}` | {} |\n", package.name, dependencies));
    }
    out
}

#[cfg(test)]
#[path = "tests/workspace_gen_tests.rs"]
mod workspace_gen_tests;
