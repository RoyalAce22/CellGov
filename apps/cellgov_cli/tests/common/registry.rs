//! Shared registry reader for the title-corpus suites.
//!
//! An integration test cannot link the CLI's manifest loader, which
//! lives in a binary crate, so this mirrors its acceptance rules:
//!
//! - tables at the root or under `[cellgov]`,
//! - one `[[bench.matrix]]` row marked `reference = true`,
//! - a `game_ver` on every cell of a stored title, and none on a
//!   firmware-shipped one.
//!
//! This reader panics on a duplicate short name, a duplicate content
//! id, or a key whose type the loader refuses.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Instruction cap for titles whose manifest does not set one.
pub const DEFAULT_BENCH_MAX_STEPS: u64 = 100_000_000;

/// The `game_ver` naming a title's base install.
#[allow(dead_code, reason = "not every suite names a game version")]
pub const BASE_GAME_VER: &str = "base";

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
    /// The cell the manifest marks `reference = true`, which is the
    /// one a suite boots and holds against its anchor.
    #[allow(dead_code, reason = "not every suite reads every field")]
    pub reference: ReferenceCell,
}

/// The `(firmware, game version)` key of a title's reference cell.
pub struct ReferenceCell {
    pub fw: String,
    /// `None` for a title shipped inside the firmware, whose version
    /// axis is the firmware's.
    pub game_ver: Option<String>,
}

impl ReferenceCell {
    /// How a failure names this cell.
    #[allow(dead_code, reason = "not every suite reports a cell by name")]
    pub fn label(&self) -> String {
        match &self.game_ver {
            Some(v) => format!("fw {} x {v}", self.fw),
            None => format!("fw {}", self.fw),
        }
    }
}

/// The table set the loader reads, at root or under `[cellgov]`.
///
/// A `[cellgov]` table holds every table of the manifest, including
/// `[[bench.matrix]]`.
fn manifest_root(doc: &toml::Value) -> &toml::Value {
    doc.get("cellgov").filter(|c| c.is_table()).unwrap_or(doc)
}

/// Whether `[source] kind` marks the title as shipped inside the
/// firmware.
///
/// Such a title carries no game-version axis.
fn is_firmware_exec(root: &toml::Value) -> bool {
    root.get("source")
        .and_then(|s| s.get("kind"))
        .and_then(toml::Value::as_str)
        == Some("firmware-exec")
}

/// The one `[[bench.matrix]]` row marked `reference = true`.
///
/// The loader refuses a manifest that marks no row or several, so every
/// manifest this suite reads has exactly one.
///
/// # Panics
///
/// Panics if the manifest:
///
/// - declares no `[[bench.matrix]]` row,
/// - marks other than one row `reference = true`, or
/// - spells `reference` as a non-boolean.
fn reference_row<'a>(path: &Path, root: &'a toml::Value) -> &'a toml::Value {
    let rows = root
        .get("bench")
        .and_then(|b| b.get("matrix"))
        .and_then(toml::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    assert!(
        !rows.is_empty(),
        "{}: no [[bench.matrix]] row. An anchor is keyed by (content id, firmware, game \
         version), so a title declaring no cell has nothing for this suite to boot or gate",
        path.display()
    );
    for row in rows {
        if let Some(v) = row.get("reference") {
            assert!(
                v.as_bool().is_some(),
                "{}: [[bench.matrix]] reference must be a boolean, got {v:?}",
                path.display()
            );
        }
    }
    let marked: Vec<&toml::Value> = rows
        .iter()
        .filter(|r| r.get("reference").and_then(toml::Value::as_bool) == Some(true))
        .collect();
    let [row] = marked.as_slice() else {
        panic!(
            "{}: [[bench.matrix]] marks {} rows reference = true; exactly one is the cell \
             this suite boots",
            path.display(),
            marked.len()
        );
    };
    row
}

/// One version key of the anchor tree's path.
///
/// Both `fw` and `game_ver` become a path segment, so this mirrors the
/// store's version-key rule.
///
/// # Panics
///
/// Panics if `v` is empty, is `..`, or holds a path separator.
fn version_key(path: &Path, what: &str, v: &str) -> String {
    assert!(
        !v.is_empty() && v != ".." && !v.contains('/') && !v.contains('\\'),
        "{}: {what} {v:?} cannot name a store directory",
        path.display()
    );
    v.to_string()
}

/// The `(fw, game_ver)` key the reference row declares.
///
/// Every cell of a title the store holds carries a `game_ver`. A title
/// shipped inside the firmware carries none.
///
/// # Panics
///
/// Panics if the row states no `fw`, or if its `game_ver` disagrees
/// with the title's source kind.
fn reference_cell(path: &Path, root: &toml::Value, row: &toml::Value) -> ReferenceCell {
    let Some(fw) = row.get("fw").and_then(toml::Value::as_str) else {
        panic!(
            "{}: the reference [[bench.matrix]] row has no fw string",
            path.display()
        );
    };
    let firmware_exec = is_firmware_exec(root);
    let game_ver = match (row.get("game_ver"), firmware_exec) {
        (None, true) => None,
        (None, false) => panic!(
            "{}: the reference [[bench.matrix]] row states no game_ver; name \"base\" or an \
             update version key",
            path.display()
        ),
        (Some(v), true) => panic!(
            "{}: the reference [[bench.matrix]] row states game_ver {v:?}, which does not \
             apply to a title shipped inside the firmware",
            path.display()
        ),
        (Some(v), false) => {
            let raw = v.as_str().unwrap_or_else(|| {
                panic!(
                    "{}: [[bench.matrix]] game_ver must be a string, got {v:?}",
                    path.display()
                )
            });
            Some(version_key(path, "game_ver", raw))
        }
    };
    ReferenceCell {
        fw: version_key(path, "fw", fw),
        game_ver,
    }
}

/// Read every registered title, accepting both manifest layouts and
/// failing loudly on duplicates or missing identity fields.
pub fn titles() -> Vec<TitleUnderTest> {
    let dir = workspace_root().join("title_manifests");
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
        let root = manifest_root(&doc);
        let title = root
            .get("title")
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
        let row = reference_row(&path, root);
        let reference = reference_cell(&path, root, row);
        let max_steps = max_steps_key(
            &path,
            row.get("bench_max_steps"),
            "the reference [[bench.matrix]] row",
        )
        .or_else(|| max_steps_key(&path, title.get("bench_max_steps"), "[title]"))
        .unwrap_or(DEFAULT_BENCH_MAX_STEPS);
        out.push(TitleUnderTest {
            short_name,
            content_id,
            max_steps,
            reference,
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

/// One `bench_max_steps` value, or `None` when the key is absent.
///
/// # Panics
///
/// Panics if the key is present but is not a non-negative integer.
fn max_steps_key(path: &Path, value: Option<&toml::Value>, what: &str) -> Option<u64> {
    let v = value?;
    Some(
        v.as_integer()
            .and_then(|n| u64::try_from(n).ok())
            .unwrap_or_else(|| {
                panic!(
                    "{}: {what} bench_max_steps must be a non-negative integer, got {v:?}",
                    path.display()
                )
            }),
    )
}

/// The committed anchor for one cell of `content_id`.
///
/// Mirrors `cellgov_cli`'s `paths::boot_anchor_path`, which an
/// integration test cannot link.
#[allow(dead_code, reason = "not every suite reads boot anchors")]
pub fn boot_anchor_path(content_id: &str, cell: &ReferenceCell) -> PathBuf {
    let dir = workspace_root()
        .join("tests")
        .join("fixtures")
        .join(content_id)
        .join("cellgov")
        .join("anchors")
        .join(format!("fw-{}", cell.fw));
    match &cell.game_ver {
        Some(v) => dir.join(v),
        None => dir,
    }
    .join("boot_summary.json")
}
