//! Convention guard: the data group of `.github/ci.sh` runs every
//! feature-gated test target.
//!
//! A `[[test]]` target with `required-features` builds only when the
//! operator turns its external-data feature on. The data group selects
//! these targets by name from two lists in `.github/ci.sh`:
//!
//! - `data_targets` holds the `cellgov_cli` targets;
//! - `install_data_targets` holds the `cellgov_install` targets.
//!
//! A target that is not in its list never runs under the `ci-data`
//! profile. The live-console suite of `runner_ps3` stays out of the data
//! group, because it drives the PS3.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Each workspace member that the data group runs, with the `ci.sh`
/// variable that lists its gated test targets.
const DATA_GROUP_LISTS: &[(&str, &str)] = &[
    ("apps/cellgov_cli", "data_targets"),
    ("apps/cellgov_install", "install_data_targets"),
];

/// The member whose gated targets drive the live console.
const LIVE_CONSOLE_MEMBER: &str = "bridges/runner_ps3";

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

/// The member paths in the root manifest's `[workspace]` list.
fn workspace_members(root: &Path) -> Vec<String> {
    let manifest = read(&root.join("Cargo.toml"));
    let list = manifest
        .split_once("members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| list)
        .expect("the root Cargo.toml has a members list");
    list.split(',')
        .map(|entry| entry.trim().trim_matches('"').to_string())
        .filter(|entry| !entry.is_empty())
        .collect()
}

/// The names of the `[[test]]` targets in `manifest` that carry
/// `required-features`.
fn gated_test_targets(manifest: &str) -> BTreeSet<String> {
    let mut gated = BTreeSet::new();
    for block in manifest.split("[[test]]").skip(1) {
        let block = block.split("\n[").next().unwrap_or(block);
        let has_key = |key: &str| block.lines().any(|l| l.trim().starts_with(key));
        let name = block
            .lines()
            .find_map(|l| l.trim().strip_prefix("name")?.split_once('='))
            .map(|(_, v)| v.trim().trim_matches('"').to_string());
        if let (Some(name), true) = (name, has_key("required-features")) {
            gated.insert(name);
        }
    }
    gated
}

/// The names that the `ci.sh` assignment `<variable>="..."` lists.
fn ci_list(ci: &str, variable: &str) -> BTreeSet<String> {
    let prefix = format!("{variable}=\"");
    let line = ci
        .lines()
        .find_map(|l| l.strip_prefix(&prefix))
        .unwrap_or_else(|| panic!("ci.sh assigns no {variable}"));
    let names: BTreeSet<String> = line
        .trim_end_matches('"')
        .split_whitespace()
        .map(str::to_string)
        .collect();
    assert!(!names.is_empty(), "ci.sh lists no name in {variable}");
    names
}

#[test]
fn every_gated_test_target_is_in_its_data_group_list() {
    let root = workspace_root();
    let ci = read(&root.join(".github/ci.sh"));
    let mut missing = Vec::new();
    let mut checked = 0;
    for member in workspace_members(&root) {
        let gated = gated_test_targets(&read(&root.join(&member).join("Cargo.toml")));
        if gated.is_empty() || member == LIVE_CONSOLE_MEMBER {
            continue;
        }
        let Some((_, variable)) = DATA_GROUP_LISTS.iter().find(|(m, _)| *m == member) else {
            missing.push(format!("{member}: no data-group list runs {gated:?}"));
            continue;
        };
        let listed = ci_list(&ci, variable);
        for target in gated {
            checked += 1;
            if !listed.contains(&target) {
                missing.push(format!("{member}: {target} is not in {variable}"));
            }
        }
    }
    assert!(checked > 0, "no gated test target was found");
    assert!(missing.is_empty(), "unlisted gated targets: {missing:#?}");
}

#[test]
fn every_data_group_list_entry_is_a_gated_test_target() {
    let root = workspace_root();
    let ci = read(&root.join(".github/ci.sh"));
    let mut stale = Vec::new();
    for (member, variable) in DATA_GROUP_LISTS {
        let gated = gated_test_targets(&read(&root.join(member).join("Cargo.toml")));
        for name in ci_list(&ci, variable) {
            if !gated.contains(&name) {
                stale.push(format!("{variable}: {name} is not a gated {member} target"));
            }
        }
    }
    assert!(stale.is_empty(), "stale data-group entries: {stale:#?}");
}
