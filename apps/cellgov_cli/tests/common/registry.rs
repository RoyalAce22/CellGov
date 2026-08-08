//! Shared registry reader for the title-corpus suites.
//!
//! Integration tests cannot link the CLI's manifest loader (binary
//! crate), so this mirrors its acceptance rules: `[title]` at the
//! root or under `[cellgov]`, and duplicate short names or content
//! ids are a loud failure rather than a silent skip -- a title that
//! vanishes from the suite must never vanish quietly.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Instruction cap for titles whose manifest does not set one.
pub const DEFAULT_BENCH_MAX_STEPS: u64 = 100_000_000;

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above apps/cellgov_cli")
        .to_path_buf()
}

/// One title's identity, read from its manifest TOML.
pub struct TitleUnderTest {
    pub short_name: String,
    #[allow(dead_code, reason = "not every suite reads every field")]
    pub content_id: String,
    #[allow(dead_code, reason = "not every suite reads every field")]
    pub max_steps: u64,
}

/// Read every registered title, accepting both manifest layouts and
/// failing loudly on duplicates or missing identity fields.
pub fn titles() -> Vec<TitleUnderTest> {
    let dir = workspace_root().join("docs/title_manifests");
    let mut out = Vec::new();
    let mut short_names: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut content_ids: BTreeMap<String, PathBuf> = BTreeMap::new();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("read manifest dir entry").path();
        let is_toml = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("toml"));
        let is_hidden = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.'));
        if !is_toml || is_hidden {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let doc: toml::Value = text
            .parse()
            .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let title = doc
            .get("title")
            .or_else(|| doc.get("cellgov").and_then(|c| c.get("title")))
            .and_then(toml::Value::as_table)
            .unwrap_or_else(|| {
                panic!(
                    "{}: no [title] table at root or under [cellgov]",
                    path.display()
                )
            });
        let get = |k: &str| {
            title
                .get(k)
                .and_then(toml::Value::as_str)
                .map(str::to_string)
        };
        let (Some(short_name), Some(content_id)) = (get("short_name"), get("content_id")) else {
            panic!(
                "{}: [title] needs short_name and content_id",
                path.display()
            );
        };
        if let Some(prev) = short_names.insert(short_name.clone(), path.clone()) {
            panic!(
                "duplicate short_name {short_name:?}: {} and {}",
                prev.display(),
                path.display()
            );
        }
        if let Some(prev) = content_ids.insert(content_id.clone(), path.clone()) {
            panic!(
                "duplicate content_id {content_id:?}: {} and {}",
                prev.display(),
                path.display()
            );
        }
        let max_steps = title
            .get("bench_max_steps")
            .and_then(toml::Value::as_integer)
            .and_then(|v| u64::try_from(v).ok())
            .unwrap_or(DEFAULT_BENCH_MAX_STEPS);
        out.push(TitleUnderTest {
            short_name,
            content_id,
            max_steps,
        });
    }
    out.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    assert!(
        !out.is_empty(),
        "no title manifests found in {}",
        dir.display()
    );
    out
}

#[allow(dead_code, reason = "not every suite reads baselines")]
pub fn baseline_path(content_id: &str) -> PathBuf {
    workspace_root().join(format!(
        "tests/fixtures/{content_id}/cellgov/boot_summary.json"
    ))
}
