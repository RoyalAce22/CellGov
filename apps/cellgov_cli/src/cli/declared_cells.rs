//! The cells the registry declares, enumerated once for every command
//! that runs them one after another.
//!
//! `dev record-anchors` measures and writes them; `boot bench --all`
//! measures and compares them. Both read the cells from the registry
//! alone: the one cell `[title] system_ver` derives and every
//! `[[bench.matrix]]` row, in declaration order. A cell with no anchor
//! is then visible; a walk over the anchor tree would omit it.

use std::path::Path;

use crate::cli::exit::die;
use crate::game::manifest::{CellKey, CheckpointTrigger, TitleManifest, TitleRegistry};
use crate::paths::{cell_checkpoint, cell_max_steps};

/// One declared cell, with the step cap and checkpoint its measurement
/// uses.
pub(crate) struct DeclaredCell {
    pub short_name: String,
    pub content_id: String,
    pub cell: CellKey,
    pub max_steps: u64,
    pub checkpoint: CheckpointTrigger,
    /// The registry's reason this cell has no measurement yet.
    pub pending: Option<String>,
}

impl DeclaredCell {
    /// How a report and a refusal name this cell.
    pub(crate) fn label(&self) -> String {
        format!("{} {}", self.short_name, self.cell.label())
    }
}

/// Every cell `title` declares, in declaration order.
pub(crate) fn declared_cells(title: &TitleManifest) -> Vec<DeclaredCell> {
    title
        .matrix
        .iter()
        .map(|cell| DeclaredCell {
            short_name: title.short_name.clone(),
            content_id: title.content_id.clone(),
            cell: cell.key.clone(),
            max_steps: cell_max_steps(title, Some(cell)),
            checkpoint: cell_checkpoint(title, Some(cell)),
            pending: cell.pending.clone(),
        })
        .collect()
}

/// Split `cells` into `(kept, pending)` by the registry's `pending`
/// marker; `keep_pending` moves the pending cells into `kept` too.
///
/// Something outside the registry stops a pending cell. A sweep that
/// measures it dies at that boot and takes every other cell of the
/// title with it.
pub(crate) fn split_pending(
    cells: Vec<DeclaredCell>,
    keep_pending: bool,
) -> (Vec<DeclaredCell>, Vec<DeclaredCell>) {
    cells
        .into_iter()
        .partition(|c| keep_pending || c.pending.is_none())
}

/// Every manifest under `dir`, ascending by short name.
pub(crate) fn read_registry(dir: &Path) -> Vec<TitleManifest> {
    let registry = TitleRegistry::scan_dir(dir)
        .unwrap_or_else(|e| die(&format!("scan registry {}: {e}", dir.display())));
    let mut out: Vec<TitleManifest> = registry.iter().cloned().collect();
    out.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    out
}

/// The one title `one` names, or every registered title.
pub(crate) fn select_titles<'a>(
    titles: &'a [TitleManifest],
    one: Option<&str>,
) -> Vec<&'a TitleManifest> {
    let Some(name) = one else {
        return titles.iter().collect();
    };
    let Some(hit) = titles.iter().find(|t| t.short_name == name) else {
        let known: Vec<&str> = titles.iter().map(|t| t.short_name.as_str()).collect();
        die(&format!(
            "unknown title {name:?}; registry has: {}",
            known.join(", ")
        ));
    };
    vec![hit]
}

/// Refuse a selection that includes a title with no declared cell.
///
/// A title with a PARAM.SFO always declares the cell its `system_ver`
/// derives. Only a title shipped inside the firmware, or built beside
/// its manifest, can reach here with nothing declared.
pub(crate) fn refuse_undeclared(selected: &[&TitleManifest]) {
    let undeclared: Vec<&str> = selected
        .iter()
        .filter(|t| t.matrix.is_empty())
        .map(|t| t.short_name.as_str())
        .collect();
    if !undeclared.is_empty() {
        die(&format!(
            "no cells declared for: {}. An anchor is keyed by (content id, firmware, game \
             version), and a title with no floor of its own declares its cells as \
             [[bench.matrix]] rows alone; with none it has nothing to record and nothing \
             for the gate to read",
            undeclared.join(", ")
        ));
    }
}

/// Narrow the declared cells to those `--fw` / `--game-ver` name, and
/// refuse a cell the registry does not declare.
///
/// `command` names the invocation in the refusal.
pub(crate) fn filter_declared(
    cells: Vec<DeclaredCell>,
    fw: Option<&str>,
    game_ver: Option<&str>,
    command: &str,
) -> Vec<DeclaredCell> {
    if fw.is_none() && game_ver.is_none() {
        return cells;
    }
    let declared: Vec<String> = cells.iter().map(DeclaredCell::label).collect();
    let kept: Vec<DeclaredCell> = cells
        .into_iter()
        .filter(|c| {
            fw.is_none_or(|f| c.cell.fw == f)
                && game_ver.is_none_or(|v| c.cell.game_ver.as_deref() == Some(v))
        })
        .collect();
    if kept.is_empty() {
        let asked = match (fw, game_ver) {
            (Some(f), Some(v)) => format!("fw {f} x {v}"),
            (Some(f), None) => format!("fw {f}"),
            (None, Some(v)) => format!("game version {v}"),
            (None, None) => unreachable!("an unfiltered selection returned above"),
        };
        die(&format!(
            "{command}: the registry declares no cell matching {asked}; declared: {}. \
             The gate reads declared cells, so an anchor recorded outside the declaration \
             would be compared against by nothing. Add the row to [[bench.matrix]] first",
            if declared.is_empty() {
                "none".to_string()
            } else {
                declared.join(", ")
            }
        ));
    }
    kept
}

#[cfg(test)]
#[path = "tests/declared_cells_tests.rs"]
mod tests;
