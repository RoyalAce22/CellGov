//! `cellgov dev workspace-gen` -- Cargo-derived architecture-map regions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;

use super::exit::CommandError;
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

pub(crate) fn run(args: &WorkspaceGenArgs) -> Result<(), CommandError> {
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT));
    let metadata = cargo_metadata()?;
    let document = std::fs::read_to_string(&output).map_err(|error| {
        CommandError::failed(format!("workspace-gen: read {}: {error}", output.display()))
    })?;
    let rendered = render_document(&document, &metadata, LAYERS)
        .map_err(|error| CommandError::failed(format!("workspace-gen: {error}")))?;
    std::fs::write(&output, rendered).map_err(|error| {
        CommandError::failed(format!(
            "workspace-gen: write {}: {error}",
            output.display()
        ))
    })?;
    println!("workspace-gen: wrote {}", output.display());
    Ok(())
}

fn cargo_metadata() -> Result<Metadata, CommandError> {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|error| {
            CommandError::failed(format!("workspace-gen: run cargo metadata: {error}"))
        })?;
    if !output.status.success() {
        return Err(CommandError::failed(format!(
            "workspace-gen: cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| {
        CommandError::failed(format!("workspace-gen: parse cargo metadata: {error}"))
    })
}

/// Each layer's name and its crates' short names, bottom to top.
type Layers<'a> = &'a [(&'a str, &'a [&'a str])];

/// Why the regions could not be rendered.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
enum WorkspaceGenError {
    /// A marker is absent or appears more than once.
    #[error("expected one {0}")]
    Marker(&'static str),
    /// The end marker comes before its start marker.
    #[error("{end} precedes {start}")]
    MarkerOrder {
        start: &'static str,
        end: &'static str,
    },
    /// A workspace member sits in no layer.
    #[error("workspace member {0} is in no layer; place it in LAYERS")]
    Unplaced(String),
    /// A layer names a crate twice, or two layers name one crate.
    #[error("the layer table places {0} more than once")]
    PlacedTwice(String),
    /// A layer names a crate the workspace does not have.
    #[error("the layer table names {0}, which is not a workspace member")]
    NotAMember(String),
    /// A crate depends on one drawn above it.
    #[error("{from} depends on {to}, which sits in a higher layer")]
    UpwardEdge { from: String, to: String },
}

fn render_document(
    document: &str,
    metadata: &Metadata,
    layers: Layers<'_>,
) -> Result<String, WorkspaceGenError> {
    let dag = render_dag(metadata, layers)?;
    let external = render_external_dependencies(metadata);
    let document = replace_region(document, DAG_START, DAG_END, &dag)?;
    replace_region(&document, EXTERNAL_START, EXTERNAL_END, &external)
}

fn replace_region(
    document: &str,
    start: &'static str,
    end: &'static str,
    body: &str,
) -> Result<String, WorkspaceGenError> {
    if document.matches(start).count() != 1 {
        return Err(WorkspaceGenError::Marker(start));
    }
    if document.matches(end).count() != 1 {
        return Err(WorkspaceGenError::Marker(end));
    }
    let start_at = document
        .find(start)
        .ok_or(WorkspaceGenError::Marker(start))?;
    let content_at = start_at + start.len();
    let end_at = document[content_at..]
        .find(end)
        .map(|offset| content_at + offset)
        .ok_or(WorkspaceGenError::MarkerOrder { start, end })?;
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

/// The layers the graph draws, bottom to top, each naming its crates by
/// short name in the order the diagram lists them.
///
/// A crate belongs to exactly one layer, and no crate depends on one in
/// a higher layer. The generator refuses a new workspace member until
/// this table places it: its layer is a decision the metadata does not
/// make.
const LAYERS: &[(&str, &[&str])] = &[
    ("ABI", &["ps3_abi"]),
    (
        "Primitives",
        &["time", "event", "mem", "dma", "sync", "effects"],
    ),
    ("Execution boundary", &["exec", "trace"]),
    ("Models and interpreters", &["lv2", "ppu", "spu"]),
    ("Runtime", &["core"]),
    (
        "Host tooling",
        &[
            "terminal", "testkit", "compare", "explore", "fuzz", "install", "boot",
        ],
    ),
    ("Binaries", &["cli", "mkelf", "rpcs3_to_observation"]),
];

/// How the diagram names a crate.
fn short_name(package: &str) -> &str {
    package.strip_prefix("cellgov_").unwrap_or(package)
}

/// The workspace graph as a layered Mermaid diagram.
///
/// It draws the transitive reduction: it leaves out each edge a longer
/// path implies, so each arrow is a dependency nothing else explains.
/// Edges list by dependent in layer order, then by dependency name.
fn render_dag(metadata: &Metadata, layers: Layers<'_>) -> Result<String, WorkspaceGenError> {
    // short name -> (layer index, position in the flattened listing)
    let mut placement: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for (position, (layer, member)) in layers
        .iter()
        .enumerate()
        .flat_map(|(layer, (_, members))| members.iter().map(move |m| (layer, *m)))
        .enumerate()
    {
        if placement.insert(member, (layer, position)).is_some() {
            return Err(WorkspaceGenError::PlacedTwice(member.to_string()));
        }
    }
    let names: BTreeSet<&str> = metadata
        .packages
        .iter()
        .map(|package| short_name(&package.name))
        .collect();
    if let Some(name) = names.iter().find(|name| !placement.contains_key(*name)) {
        return Err(WorkspaceGenError::Unplaced(name.to_string()));
    }
    if let Some(member) = placement.keys().find(|member| !names.contains(*member)) {
        return Err(WorkspaceGenError::NotAMember(member.to_string()));
    }

    // Each dependent with its direct workspace dependencies. The checks
    // above place every name, so `place` never falls back.
    let mut direct: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for package in &metadata.packages {
        let from = short_name(&package.name);
        let deps = direct.entry(from).or_default();
        for dependency in normal_dependencies(package) {
            let to = short_name(&dependency.name);
            if dependency.path.is_some() && names.contains(to) {
                deps.insert(to);
            }
        }
    }
    let place = |name: &str| placement.get(name).copied().unwrap_or((0, 0));
    for (from, deps) in &direct {
        if let Some(to) = deps.iter().find(|to| place(to).0 > place(from).0) {
            return Err(WorkspaceGenError::UpwardEdge {
                from: from.to_string(),
                to: to.to_string(),
            });
        }
    }

    let mut dependents: Vec<(&str, &BTreeSet<&str>)> =
        direct.iter().map(|(from, deps)| (*from, deps)).collect();
    dependents.sort_by_key(|(name, _)| place(name).1);
    let mut edges = Vec::new();
    for (from, deps) in dependents {
        for to in deps {
            let implied = deps
                .iter()
                .any(|via| via != to && reaches(&direct, via, to));
            if !implied {
                edges.push((from, *to));
            }
        }
    }

    let mut out = String::from("\n```mermaid\ngraph BT\n");
    for (layer, members) in layers {
        out.push_str(&format!(
            "  subgraph {layer}\n    {}\n  end\n",
            members.join("; ")
        ));
    }
    out.push('\n');
    for (from, to) in edges {
        out.push_str(&format!("  {to} --> {from}\n"));
    }
    out.push_str("```\n");
    Ok(out)
}

/// Whether `from` depends on `to` through any path.
fn reaches(direct: &BTreeMap<&str, BTreeSet<&str>>, from: &str, to: &str) -> bool {
    let mut stack = vec![from];
    let mut seen = BTreeSet::new();
    while let Some(at) = stack.pop() {
        if at == to {
            return true;
        }
        if seen.insert(at) {
            if let Some(next) = direct.get(at) {
                stack.extend(next.iter().copied());
            }
        }
    }
    false
}

/// The direct external dependencies, as a column-aligned table.
fn render_external_dependencies(metadata: &Metadata) -> String {
    let rows: Vec<(String, String)> = metadata
        .packages
        .iter()
        .map(|package| {
            let dependencies: BTreeSet<_> = normal_dependencies(package)
                .filter(|dependency| dependency.source.is_some())
                .map(|dependency| dependency.name.as_str())
                .collect();
            let dependencies = if dependencies.is_empty() {
                "none".to_string()
            } else {
                dependencies.into_iter().collect::<Vec<_>>().join(", ")
            };
            (format!("`{}`", package.name), dependencies)
        })
        .collect();
    let header = ("Crate", "Direct external dependencies");
    let crate_width = rows
        .iter()
        .map(|(name, _)| name.len())
        .chain([header.0.len()])
        .max()
        .unwrap_or(0);
    let deps_width = rows
        .iter()
        .map(|(_, deps)| deps.len())
        .chain([header.1.len()])
        .max()
        .unwrap_or(0);
    let mut out = format!(
        "\n| {:<crate_width$} | {:<deps_width$} |\n| {} | {} |\n",
        header.0,
        header.1,
        "-".repeat(crate_width),
        "-".repeat(deps_width),
    );
    for (name, deps) in rows {
        out.push_str(&format!("| {name:<crate_width$} | {deps:<deps_width$} |\n"));
    }
    out
}

#[cfg(test)]
#[path = "tests/workspace_gen_tests.rs"]
mod workspace_gen_tests;
