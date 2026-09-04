//! The store's read surface: `status`, and the `list` / `show` /
//! `verify` verbs under `firmware` and `title`.
//!
//! Every command here reads and reports; none writes. Results go to
//! stdout -- one JSON document under `--format json`, aligned columns
//! otherwise. Warnings and hints go to stderr.

mod collect;
mod list;
mod model;
mod status;
mod verify;

use std::path::{Path, PathBuf};

use crate::cli::exit::die;
use crate::cli::parse::OutputFormat;
use crate::composition::inventory::StoreInventory;
use crate::game::manifest::TitleRegistry;

pub(crate) use list::{firmware_list, firmware_show, title_list, title_show};
pub(crate) use status::status;
pub(crate) use verify::{firmware_verify, title_verify};

use collect::StoreView;

/// Read the store under `root`, or die naming what refused.
fn view(root: &Path) -> StoreView {
    let inventory = StoreInventory::read(root).unwrap_or_else(|e| die(&e.to_string()));
    let registry_dir = Path::new(crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR);
    let registry = TitleRegistry::scan_dir(registry_dir)
        .unwrap_or_else(|e| die(&format!("title registry: {e}")));
    // The registry directory resolves against the working directory, and
    // an absent one reads as a registry that declares nothing.
    if registry.is_empty() && !registry_dir.is_dir() {
        eprintln!(
            "warning: no title registry directory {} under the working directory; nothing \
             declares a title, so every installed title reads as an orphan and no cell is named",
            registry_dir.display()
        );
    }
    StoreView {
        root: root.to_path_buf(),
        inventory,
        registry,
        fixtures: crate::paths::fixtures_dir(&crate::paths::workspace_root()),
    }
}

/// Print one document to stdout as JSON, or die naming the field that
/// would not serialize.
fn emit_json<T: serde::Serialize>(doc: &T) {
    match serde_json::to_string_pretty(doc) {
        Ok(text) => println!("{text}"),
        Err(e) => die(&format!("rendering the report as JSON: {e}")),
    }
}

fn emit<T: serde::Serialize>(format: OutputFormat, doc: &T, human: impl FnOnce()) {
    match format {
        OutputFormat::Json => emit_json(doc),
        OutputFormat::Human => human(),
    }
}

/// What a size walk found, and what it could not read.
///
/// `bytes` is a floor whenever `unreadable` is non-zero: the walk counts
/// every refusal it meets.
#[derive(Debug, Default)]
struct TreeSize {
    /// Total bytes of the regular files the walk read.
    bytes: u64,
    /// Directories and entries the walk could not read.
    unreadable: usize,
}

/// Bytes every regular file under `dir` holds, and the paths refused.
///
/// An absent directory holds nothing; one that refuses for any other
/// reason counts as unreadable.
fn tree_bytes(dir: &Path) -> TreeSize {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return TreeSize::default(),
        Err(_) => {
            return TreeSize {
                bytes: 0,
                unreadable: 1,
            }
        }
    };
    let mut total = TreeSize::default();
    for entry in entries {
        let Ok(entry) = entry else {
            total.unreadable += 1;
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            total.unreadable += 1;
            continue;
        };
        if meta.is_dir() {
            let child = tree_bytes(&entry.path());
            total.bytes += child.bytes;
            total.unreadable += child.unreadable;
        } else if meta.is_file() {
            total.bytes += meta.len();
        }
    }
    total
}

/// A byte count in the unit an operator reads at a glance.
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    // One decimal place rounds a figure just under the next unit up to
    // `1024.0`, which reads as a quantity that unit already covers.
    if unit + 1 < UNITS.len() && (value * 10.0).round() >= 10_240.0 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// A list of keys as a refusal renders it.
fn key_list(keys: &[String]) -> String {
    if keys.is_empty() {
        "<none>".to_string()
    } else {
        keys.join(", ")
    }
}

/// Where the read commands look for the store, given `--vfs-root`.
pub(crate) fn store_root(vfs_flag: Option<&Path>) -> PathBuf {
    crate::cli::keys::install_root_of(&crate::cli::title::resolve_ps3_vfs_root(vfs_flag))
}

#[cfg(test)]
#[path = "tests/read_tests.rs"]
mod tests;
