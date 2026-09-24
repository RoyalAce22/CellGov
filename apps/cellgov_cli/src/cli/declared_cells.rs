//! The cells the registry declares, enumerated once for every command
//! that runs them one after another.
//!
//! `dev record-anchors` measures and writes them; `boot bench --all`
//! measures and compares them. Both read the cells from the registry
//! alone: the one cell `[title] system_ver` derives and every
//! `[[bench.matrix]]` row, in declaration order. A cell with no anchor
//! is then visible; a walk over the anchor tree would omit it.

use std::path::Path;

use crate::cli::exit::CommandError;
use crate::paths::cell_max_steps;
use cellgov_boot::manifest::{CellKey, CheckpointTrigger, TitleManifest, TitleRegistry};

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
            checkpoint: title.cell_checkpoint(Some(cell)),
            pending: cell.pending.clone(),
        })
        .collect()
}

/// Split `cells` into `(kept, pending)` by the registry's `pending`
/// marker; `keep_pending` moves the pending cells into `kept` too.
///
/// An external constraint stops a pending cell. A sweep that measures
/// it stops at that boot and skips the title's remaining cells.
pub(crate) fn split_pending(
    cells: Vec<DeclaredCell>,
    keep_pending: bool,
) -> (Vec<DeclaredCell>, Vec<DeclaredCell>) {
    cells
        .into_iter()
        .partition(|c| keep_pending || c.pending.is_none())
}

/// Every manifest under `dir`, ascending by short name.
///
/// # Errors
///
/// Returns an error if the registry cannot be read.
pub(crate) fn read_registry(dir: &Path) -> Result<Vec<TitleManifest>, CommandError> {
    let registry = TitleRegistry::scan_dir(dir).map_err(|error| {
        CommandError::failed(format!("scan registry {}: {error}", dir.display()))
    })?;
    let mut out: Vec<TitleManifest> = registry.iter().cloned().collect();
    out.sort_by(|a, b| a.short_name.cmp(&b.short_name));
    Ok(out)
}

/// The one title `one` names, or every registered title.
///
/// # Errors
///
/// Returns an error if `one` does not name a registered title.
pub(crate) fn select_titles<'a>(
    titles: &'a [TitleManifest],
    one: Option<&str>,
) -> Result<Vec<&'a TitleManifest>, CommandError> {
    let Some(name) = one else {
        return Ok(titles.iter().collect());
    };
    let Some(hit) = titles.iter().find(|t| t.short_name == name) else {
        let known: Vec<&str> = titles.iter().map(|t| t.short_name.as_str()).collect();
        return Err(CommandError::failed(format!(
            "unknown title {name:?}; registry has: {}",
            known.join(", ")
        )));
    };
    Ok(vec![hit])
}

/// Refuse a selection that includes a title with no declared cell.
///
/// A title with a PARAM.SFO always declares the cell its `system_ver`
/// derives. Only a title shipped inside the firmware, or built beside
/// its manifest, can reach here with nothing declared.
///
/// # Errors
///
/// Returns an error if a selected title declares no cell.
pub(crate) fn refuse_undeclared(selected: &[&TitleManifest]) -> Result<(), CommandError> {
    let undeclared: Vec<&str> = selected
        .iter()
        .filter(|t| t.matrix.is_empty())
        .map(|t| t.short_name.as_str())
        .collect();
    if !undeclared.is_empty() {
        return Err(CommandError::failed(format!(
            "no cells declared for: {}. An anchor is keyed by (content id, firmware, game \
             version), and a title with no floor of its own declares its cells as \
             [[bench.matrix]] rows alone; with none it has nothing to record and nothing \
             for the gate to read",
            undeclared.join(", ")
        )));
    }
    Ok(())
}

/// Narrow the declared cells to those `--fw` / `--game-ver` name, and
/// refuse a cell the registry does not declare.
///
/// `command` names the invocation in the refusal.
///
/// # Errors
///
/// Returns an error if no declared cell matches the filters.
pub(crate) fn filter_declared(
    cells: Vec<DeclaredCell>,
    fw: Option<&str>,
    game_ver: Option<&str>,
    command: &str,
) -> Result<Vec<DeclaredCell>, CommandError> {
    if fw.is_none() && game_ver.is_none() {
        return Ok(cells);
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
            (None, None) => {
                return Err(CommandError::failed(format!(
                    "{command}: an unfiltered cell selection unexpectedly became empty"
                )))
            }
        };
        return Err(CommandError::failed(format!(
            "{command}: the registry declares no cell matching {asked}; declared: {}. \
             The gate reads declared cells, so an anchor recorded outside the declaration \
             would be compared against by nothing. Add the row to [[bench.matrix]] first",
            if declared.is_empty() {
                "none".to_string()
            } else {
                declared.join(", ")
            }
        )));
    }
    Ok(kept)
}

#[cfg(test)]
#[path = "tests/declared_cells_tests.rs"]
mod tests;
