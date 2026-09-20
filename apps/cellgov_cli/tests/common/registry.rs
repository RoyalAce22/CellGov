//! Shared registry reader for the installed-title-tests suites.
//!
//! An integration test cannot link the CLI's manifest loader, which
//! lives in a binary crate, so this mirrors its acceptance rules:
//!
//! - tables at the root or under `[cellgov]`,
//! - `[title] system_ver` on every title with a PARAM.SFO; it derives
//!   the reference cell `(system_ver, base)`. None on a firmware-shipped
//!   title, whose `[[bench.matrix]]` rows are its whole declaration,
//! - a `game_ver` on every row of a stored title, and none on a
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

/// One title at the cell a suite boots and holds against its anchor.
///
/// A game title yields one of these, at its reference cell. A title
/// shipped inside the firmware yields one per declared row; the suites
/// gate every such row alike.
pub struct TitleUnderTest {
    pub short_name: String,
    #[allow(dead_code, reason = "not every suite reads every field")]
    pub content_id: String,
    #[allow(dead_code, reason = "not every suite reads every field")]
    pub max_steps: u64,
    /// The cell this entry boots.
    #[allow(dead_code, reason = "not every suite reads every field")]
    pub reference: ReferenceCell,
}

/// The `(firmware, game version)` key of a gated cell.
pub struct ReferenceCell {
    pub fw: String,
    /// `None` for a title shipped inside the firmware, whose version
    /// axis is the firmware's.
    pub game_ver: Option<String>,
    /// Why the cell carries no committed measurement yet, when the
    /// manifest states a reason. `None` means the cell must carry an
    /// anchor.
    #[allow(dead_code, reason = "not every suite reads every field")]
    pub pending: Option<String>,
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
/// Such a title carries no game-version axis and no floor.
fn is_firmware_exec(root: &toml::Value) -> bool {
    root.get("source")
        .and_then(|s| s.get("kind"))
        .and_then(toml::Value::as_str)
        == Some("firmware-exec")
}

/// The `[[bench.matrix]]` rows, or none when the table is absent.
///
/// # Panics
///
/// Panics if `matrix` is present and is not an array.
fn matrix_rows<'a>(path: &Path, root: &'a toml::Value) -> &'a [toml::Value] {
    match root.get("bench").and_then(|b| b.get("matrix")) {
        None => &[],
        Some(v) => v.as_array().map(Vec::as_slice).unwrap_or_else(|| {
            panic!(
                "{}: [bench] matrix must be an array of tables, got {v:?}",
                path.display()
            )
        }),
    }
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

/// The `pending` reason a row states, when it states one.
///
/// # Panics
///
/// Panics if the key is present and is not a non-empty string.
fn pending_of(path: &Path, row: &toml::Value) -> Option<String> {
    row.get("pending").map(|v| {
        let reason = v.as_str().unwrap_or_else(|| {
            panic!(
                "{}: [[bench.matrix]] pending must be a string stating why, got {v:?}",
                path.display()
            )
        });
        assert!(
            !reason.trim().is_empty(),
            "{}: [[bench.matrix]] pending is empty; it states why the cell cannot be \
             measured yet, and an empty reason names nothing",
            path.display()
        );
        reason.to_string()
    })
}

/// The `fw` a row names.
///
/// # Panics
///
/// Panics if the row states no `fw` string.
fn fw_of(path: &Path, row: &toml::Value) -> String {
    let Some(fw) = row.get("fw").and_then(toml::Value::as_str) else {
        panic!(
            "{}: a [[bench.matrix]] row has no fw string",
            path.display()
        );
    };
    version_key(path, "fw", fw)
}

/// The `game_ver` a row of a stored title names.
///
/// # Panics
///
/// Panics if the row states no `game_ver` string.
fn game_ver_of(path: &Path, row: &toml::Value) -> String {
    let Some(v) = row.get("game_ver") else {
        panic!(
            "{}: a [[bench.matrix]] row states no game_ver; name \"base\" or an update \
             version key",
            path.display()
        );
    };
    let raw = v.as_str().unwrap_or_else(|| {
        panic!(
            "{}: [[bench.matrix]] game_ver must be a string, got {v:?}",
            path.display()
        )
    });
    version_key(path, "game_ver", raw)
}

/// The reference cell `[title] system_ver` derives for a stored title:
/// the floor times the base install. It carries the `pending` of a row
/// that repeats it, if any.
///
/// # Panics
///
/// Panics if:
///
/// - the title states no `system_ver` string,
/// - `system_ver` uses the PARAM.SFO spelling, or
/// - a row states no `fw` or `game_ver` string.
#[allow(dead_code, reason = "not every registry suite reads reference cells")]
fn derived_cell(path: &Path, root: &toml::Value, title: &toml::Table) -> ReferenceCell {
    let Some(system_ver) = title.get("system_ver").and_then(toml::Value::as_str) else {
        panic!(
            "{}: [title] system_ver is required on a title with a PARAM.SFO; it is the \
             PS3_SYSTEM_VER the table states, as a firmware version key, and derives the \
             cell this suite boots",
            path.display()
        );
    };
    // The loader refuses the PARAM.SFO spelling (`01.5000`): it names a
    // firmware directory nothing installs.
    assert!(
        cellgov_install::system_ver::firmware_version_key(system_ver).is_err(),
        "{}: [title] system_ver {system_ver:?} is spelled the way PARAM.SFO spells it; write \
         the store's version key ({})",
        path.display(),
        cellgov_install::system_ver::firmware_version_key(system_ver).unwrap_or_default()
    );
    let fw = version_key(path, "system_ver", system_ver);
    let pending = matrix_rows(path, root)
        .iter()
        .find(|row| fw_of(path, row) == fw && game_ver_of(path, row) == BASE_GAME_VER)
        .and_then(|row| pending_of(path, row));
    ReferenceCell {
        fw,
        game_ver: Some(BASE_GAME_VER.to_string()),
        pending,
    }
}

/// One registered manifest, parsed and checked for identity.
struct Parsed {
    path: PathBuf,
    doc: toml::Value,
    short_name: String,
    content_id: String,
}

impl Parsed {
    fn root(&self) -> &toml::Value {
        manifest_root(&self.doc)
    }

    fn title(&self) -> &toml::Table {
        self.root()
            .get("title")
            .and_then(toml::Value::as_table)
            .expect("a parsed manifest carries the [title] table it was checked for")
    }

    /// The cap for every cell of this title, unless a row overrides it.
    fn title_max_steps(&self) -> Option<u64> {
        max_steps_key(&self.path, self.title().get("bench_max_steps"), "[title]")
    }
}

/// Every registered manifest, in either layout, sorted by short name.
fn parsed_manifests() -> Vec<Parsed> {
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
        let doc: toml::Value =
            toml::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let title = manifest_root(&doc)
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
        out.push(Parsed {
            path,
            doc,
            short_name,
            content_id,
        });
    }
    assert!(
        !out.is_empty(),
        "no title manifests found in {}",
        dir.display()
    );
    out.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    out
}

/// Every registered game title at its reference cell, by short name.
///
/// A title shipped inside the firmware has no reference cell and is not
/// here; see [`firmware_exec_titles`].
#[allow(dead_code, reason = "not every registry suite boots game titles")]
pub fn titles() -> Vec<TitleUnderTest> {
    parsed_manifests()
        .into_iter()
        .filter(|m| !is_firmware_exec(m.root()))
        .map(|m| {
            let reference = derived_cell(&m.path, m.root(), m.title());
            // A row that repeats the derived cell may override the cap.
            let row_cap = matrix_rows(&m.path, m.root())
                .iter()
                .find(|row| {
                    fw_of(&m.path, row) == reference.fw
                        && game_ver_of(&m.path, row) == BASE_GAME_VER
                })
                .and_then(|row| {
                    max_steps_key(
                        &m.path,
                        row.get("bench_max_steps"),
                        "the [[bench.matrix]] row repeating the derived cell",
                    )
                });
            TitleUnderTest {
                max_steps: row_cap
                    .or_else(|| m.title_max_steps())
                    .unwrap_or(DEFAULT_BENCH_MAX_STEPS),
                short_name: m.short_name,
                content_id: m.content_id,
                reference,
            }
        })
        .collect()
}

/// Preserves manifest and row declaration order.
///
/// # Panics
///
/// Panics if a stored title's row has no game version, or a
/// firmware-shipped title's row has one.
#[allow(
    dead_code,
    reason = "only the anchor-structure suite reads every matrix row"
)]
pub fn declared_cells() -> Vec<TitleUnderTest> {
    let mut out = Vec::new();
    for m in parsed_manifests() {
        let firmware_exec = is_firmware_exec(m.root());
        for row in matrix_rows(&m.path, m.root()) {
            let game_ver = if firmware_exec {
                if let Some(v) = row.get("game_ver") {
                    panic!(
                        "{}: a [[bench.matrix]] row states game_ver {v:?}, which does not apply \
                         to a title shipped inside the firmware",
                        m.path.display()
                    );
                }
                None
            } else {
                Some(game_ver_of(&m.path, row))
            };
            out.push(TitleUnderTest {
                short_name: m.short_name.clone(),
                content_id: m.content_id.clone(),
                max_steps: max_steps_key(
                    &m.path,
                    row.get("bench_max_steps"),
                    "a [[bench.matrix]] row",
                )
                .or_else(|| m.title_max_steps())
                .unwrap_or(DEFAULT_BENCH_MAX_STEPS),
                reference: ReferenceCell {
                    fw: fw_of(&m.path, row),
                    game_ver,
                    pending: pending_of(&m.path, row),
                },
            });
        }
    }
    out
}

/// Every declared cell of every registered title shipped inside the
/// firmware, one entry per cell, by short name then declaration order.
///
/// # Panics
///
/// Panics if a firmware-shipped manifest carries `[title] system_ver`,
/// or a row of one states a `game_ver`.
#[allow(dead_code, reason = "not every suite boots the system software")]
pub fn firmware_exec_titles() -> Vec<TitleUnderTest> {
    let mut out = Vec::new();
    for m in parsed_manifests()
        .into_iter()
        .filter(|m| is_firmware_exec(m.root()))
    {
        assert!(
            m.title().get("system_ver").is_none(),
            "{}: [title] system_ver does not apply to a title shipped inside the firmware; \
             it has no PARAM.SFO to state a floor",
            m.path.display()
        );
        for row in matrix_rows(&m.path, m.root()) {
            if let Some(v) = row.get("game_ver") {
                panic!(
                    "{}: a [[bench.matrix]] row states game_ver {v:?}, which does not apply \
                     to a title shipped inside the firmware",
                    m.path.display()
                );
            }
            out.push(TitleUnderTest {
                short_name: m.short_name.clone(),
                content_id: m.content_id.clone(),
                max_steps: max_steps_key(
                    &m.path,
                    row.get("bench_max_steps"),
                    "a [[bench.matrix]] row",
                )
                .or_else(|| m.title_max_steps())
                .unwrap_or(DEFAULT_BENCH_MAX_STEPS),
                reference: ReferenceCell {
                    fw: fw_of(&m.path, row),
                    game_ver: None,
                    pending: pending_of(&m.path, row),
                },
            });
        }
    }
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
