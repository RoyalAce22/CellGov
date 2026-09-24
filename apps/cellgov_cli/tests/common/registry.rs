//! The title registry, as the installed-title-tests suites read it.
//!
//! `cellgov_boot::manifest::TitleRegistry` loads and checks every
//! manifest under `title_manifests/`, and `cellgov_boot::manifest`
//! places each cell's anchor. This module only shapes that into the
//! cells a suite boots: a game title at its reference cell, the floor
//! its `system_ver` derives times the base install, and every declared
//! cell of a title shipped inside the firmware.

use std::path::{Path, PathBuf};

use cellgov_boot::manifest::{
    boot_anchor_path_in, CellKey, MatrixCell, TitleManifest, TitleRegistry,
};

/// Instruction cap for titles whose manifest does not set one.
#[allow(unused_imports, reason = "not every suite names the default cap")]
pub use cellgov_boot::manifest::DEFAULT_BENCH_MAX_STEPS;

#[allow(unused_imports, reason = "not every suite names a game version")]
pub use cellgov_boot::manifest::BASE_GAME_VER;

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
        self.key().label()
    }

    fn key(&self) -> CellKey {
        CellKey {
            fw: self.fw.clone(),
            game_ver: self.game_ver.clone(),
        }
    }
}

/// Every registered manifest, sorted by short name.
///
/// # Panics
///
/// Panics when the loader refuses a manifest, or when the registry
/// holds none.
fn manifests() -> Vec<TitleManifest> {
    let dir = workspace_root().join("title_manifests");
    let registry = TitleRegistry::scan_dir(&dir)
        .unwrap_or_else(|e| panic!("load the title registry under {}: {e}", dir.display()));
    let mut manifests: Vec<TitleManifest> = registry.iter().cloned().collect();
    assert!(
        !manifests.is_empty(),
        "no title manifests found in {}",
        dir.display()
    );
    manifests.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    manifests
}

/// `m` at the cell `key` names, measured as the loader measures it.
fn at_cell(m: &TitleManifest, key: CellKey, cell: Option<&MatrixCell>) -> TitleUnderTest {
    TitleUnderTest {
        short_name: m.short_name.clone(),
        content_id: m.content_id.clone(),
        max_steps: m.cell_max_steps(cell),
        reference: ReferenceCell {
            fw: key.fw,
            game_ver: key.game_ver,
            pending: cell.and_then(|c| c.pending.clone()),
        },
    }
}

/// Every registered game title at its reference cell, by short name.
///
/// A title shipped inside the firmware has no reference cell and is not
/// here; see [`firmware_exec_titles`].
#[allow(dead_code, reason = "not every registry suite boots game titles")]
pub fn titles() -> Vec<TitleUnderTest> {
    manifests()
        .iter()
        .filter(|m| !m.ships_in_firmware())
        .map(|m| {
            let key = m.reference_key().unwrap_or_else(|| {
                panic!(
                    "{}: the loader accepted a game title with no reference cell",
                    m.short_name
                )
            });
            let cell = m.cell(&key);
            at_cell(m, key, cell)
        })
        .collect()
}

/// Every declared cell of every registered title, by short name then
/// declaration order.
#[allow(
    dead_code,
    reason = "only the anchor-structure suite reads every matrix row"
)]
pub fn declared_cells() -> Vec<TitleUnderTest> {
    manifests()
        .iter()
        .flat_map(|m| {
            m.matrix
                .iter()
                .map(move |cell| at_cell(m, cell.key.clone(), Some(cell)))
        })
        .collect()
}

/// Every declared cell of every registered title shipped inside the
/// firmware, one entry per cell, by short name then declaration order.
#[allow(dead_code, reason = "not every suite boots the system software")]
pub fn firmware_exec_titles() -> Vec<TitleUnderTest> {
    manifests()
        .iter()
        .filter(|m| m.ships_in_firmware())
        .flat_map(|m| {
            m.matrix
                .iter()
                .map(move |cell| at_cell(m, cell.key.clone(), Some(cell)))
        })
        .collect()
}

/// The committed anchor for one cell of `content_id`.
#[allow(dead_code, reason = "not every suite reads boot anchors")]
pub fn boot_anchor_path(content_id: &str, cell: &ReferenceCell) -> PathBuf {
    boot_anchor_path_in(
        &workspace_root().join("tests").join("fixtures"),
        content_id,
        &cell.key(),
    )
}
