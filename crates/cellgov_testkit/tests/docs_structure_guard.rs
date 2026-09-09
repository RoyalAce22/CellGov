//! Convention guards on the documentation maps.
//!
//! - Every relative link in `README.md` and in the architecture index
//!   resolves to a file or directory in the tree.
//! - The architecture index's map names exactly the documents beside
//!   it: a new document gets a row, and a deleted one loses its row.
//! - The workspace document's per-crate table names exactly the
//!   workspace members: a new crate gets a row, and a removed one loses
//!   its row.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Floor on the links the resolver checks. Both documents carry a map
/// of the tree; a collapse to a handful means the parser stopped
/// recognising the link form.
const MIN_LINKS: usize = 15;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Targets of every `[text](target)` markdown link in `text`, with the
/// 1-based line each sits on. A target with a scheme or a bare
/// fragment is not a tree path and is left out.
fn relative_links(text: &str) -> Vec<(usize, String)> {
    let mut links = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let mut rest = line;
        while let Some(at) = rest.find("](") {
            let after = &rest[at + 2..];
            let Some(end) = after.find(')') else { break };
            let target = after[..end].split_whitespace().next().unwrap_or_default();
            let target = target.split('#').next().unwrap_or_default();
            if !target.is_empty() && !target.contains("://") && !target.starts_with("mailto:") {
                links.push((n + 1, target.to_string()));
            }
            rest = &after[end + 1..];
        }
    }
    links
}

/// The first link target in each table row of the section headed
/// `heading`, up to the next heading.
fn table_links_under(text: &str, heading: &str) -> Vec<String> {
    let mut in_section = false;
    let mut found = Vec::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            in_section = line.trim() == heading;
            continue;
        }
        if in_section && line.starts_with('|') {
            if let Some(first) = relative_links(line).into_iter().next() {
                found.push(first.1);
            }
        }
    }
    found
}

/// The backticked name opening each row of the table under `heading`.
fn table_names_under(text: &str, heading: &str) -> Vec<String> {
    let mut in_section = false;
    let mut found = Vec::new();
    for line in text.lines() {
        if line.starts_with("## ") {
            in_section = line.trim() == heading;
            continue;
        }
        if in_section && line.starts_with("| `") {
            if let Some(name) = line[3..].split('`').next() {
                found.push(name.to_string());
            }
        }
    }
    found
}

/// Workspace member crate names from the root manifest's `members`
/// list: the last path component of each entry.
fn workspace_members(root: &Path) -> BTreeSet<String> {
    let manifest = read(&root.join("Cargo.toml"));
    let after = manifest
        .split_once("members = [")
        .map(|(_, rest)| rest)
        .expect("the root manifest lists workspace members");
    let list = after.split(']').next().unwrap_or_default();
    list.split(',')
        .filter_map(|entry| entry.trim().strip_prefix('"')?.strip_suffix('"'))
        .filter_map(|path| path.rsplit('/').next())
        .map(ToString::to_string)
        .collect()
}

#[test]
fn the_link_parser_reads_relative_targets_only() {
    let text = "see [a](docs/a.md) and [b](docs/b.md#part) and [c](https://x.y/z)\n\
                a [d](#anchor) fragment, [e](mailto:x@y), [f](../up/f.md \"title\")\n";
    assert_eq!(
        relative_links(text),
        vec![
            (1, "docs/a.md".to_string()),
            (1, "docs/b.md".to_string()),
            (2, "../up/f.md".to_string()),
        ]
    );
}

#[test]
fn the_table_parsers_read_one_section_and_stop_at_the_next_heading() {
    let text = "## Map\n\
                | Document | Covers |\n\
                | --- | --- |\n\
                | [x.md](x.md) | X. |\n\
                | [y.md](y.md) | Y. |\n\
                \n\
                ## Other\n\
                | [z.md](z.md) | Z. |\n\
                | `crate_a` | A. |\n\
                ## Per-crate responsibilities\n\
                | Crate | Responsibility |\n\
                | `crate_b` | B. |\n\
                | `bridges/crate_c` | C. |\n";
    assert_eq!(table_links_under(text, "## Map"), ["x.md", "y.md"]);
    assert_eq!(
        table_names_under(text, "## Per-crate responsibilities"),
        ["crate_b", "bridges/crate_c"]
    );
}

#[test]
fn every_relative_link_in_the_readme_and_the_architecture_index_resolves() {
    let root = workspace_root();
    let mut checked = 0usize;
    let mut dangling = Vec::new();
    for rel in ["README.md", "docs/architecture/README.md"] {
        let file = root.join(rel);
        let base = file.parent().expect("a file has a parent");
        for (line, target) in relative_links(&read(&file)) {
            checked += 1;
            if !base.join(&target).exists() {
                dangling.push(format!("  {rel}:{line}  {target}\n"));
            }
        }
    }
    assert!(
        checked >= MIN_LINKS,
        "gate went vacuous: only {checked} relative link(s) found"
    );
    assert!(
        dangling.is_empty(),
        "{} link(s) point at nothing in the tree:\n{}",
        dangling.len(),
        dangling.concat()
    );
}

#[test]
fn the_architecture_map_names_exactly_the_documents_beside_it() {
    let root = workspace_root();
    let dir = root.join("docs").join("architecture");
    let index = read(&dir.join("README.md"));
    let mapped: BTreeSet<String> = table_links_under(&index, "## Map").into_iter().collect();
    let mut present = BTreeSet::new();
    let entries =
        fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", dir.display()))
            .path();
        let name = path.file_name().map(|n| n.to_string_lossy().to_string());
        if let Some(name) = name {
            if name.ends_with(".md") && name != "README.md" {
                present.insert(name);
            }
        }
    }
    assert!(!mapped.is_empty(), "the map table lists no document");
    let unmapped: Vec<&String> = present.difference(&mapped).collect();
    let missing: Vec<&String> = mapped.difference(&present).collect();
    assert!(
        unmapped.is_empty() && missing.is_empty(),
        "docs/architecture/README.md's map and the directory disagree: documents with no \
         row {unmapped:?}; rows with no document {missing:?}"
    );
}

#[test]
fn the_per_crate_table_names_exactly_the_workspace_members() {
    let root = workspace_root();
    let workspace = read(&root.join("docs").join("architecture").join("workspace.md"));
    let listed: BTreeSet<String> = table_names_under(&workspace, "## Per-crate responsibilities")
        .into_iter()
        .filter_map(|name| name.rsplit('/').next().map(ToString::to_string))
        .collect();
    let members = workspace_members(&root);
    assert!(
        members.len() >= 10,
        "only {} workspace member(s) parsed",
        members.len()
    );
    let unlisted: Vec<&String> = members.difference(&listed).collect();
    let extra: Vec<&String> = listed.difference(&members).collect();
    assert!(
        unlisted.is_empty() && extra.is_empty(),
        "docs/architecture/workspace.md's per-crate table and the workspace members \
         disagree: members with no row {unlisted:?}; rows naming no member {extra:?}"
    );
}
