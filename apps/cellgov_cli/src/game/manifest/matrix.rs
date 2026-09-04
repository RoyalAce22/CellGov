//! The bench matrix: which `(firmware, game version)` cells a title
//! declares.
//!
//! The registry declares every cell. The gate and the generated
//! documents read the declared set; nothing here enumerates the store.

use std::collections::BTreeSet;
use std::path::Path;

use cellgov_install::store::VersionKey;

use super::checkpoint::CheckpointTrigger;
use super::loader::{mirror_makes_checkpoint_unreachable, parse_checkpoint, ManifestError};
use super::model::GameSource;
use super::schema::ManifestMatrixRow;

/// The `game_ver` that names a title's base install; the `--game-ver`
/// flag selects that install by this value.
pub const BASE_GAME_VER: &str = "base";

/// What the registry declares a cell to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray)]
pub enum CellExpectation {
    /// The cell expects convergence, so a divergence names the next
    /// implementation target.
    Frontier,
    /// The cell observes an incompatibility: the datum is the error
    /// the guest gets.
    Probe,
}

impl CellExpectation {
    /// Wire form for `expect = "..."`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Frontier => "frontier",
            Self::Probe => "probe",
        }
    }

    /// Inverse of [`Self::label`].
    fn from_label(s: &str) -> Option<Self> {
        use strum::VariantArray;
        Self::VARIANTS.iter().find(|v| v.label() == s).copied()
    }
}

/// One declared cell: a title at one firmware and one game version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixCell {
    /// Firmware version key of a `vfs/firmware/<key>/` entry.
    pub fw: String,
    /// [`BASE_GAME_VER`] or an update version key. `None` only for a
    /// title shipped inside the firmware, whose version axis is the
    /// firmware's.
    pub game_ver: Option<String>,
    /// True on the one cell whose measurement the headline row renders.
    pub reference: bool,
    pub expect: CellExpectation,
    /// Overrides the title-level cap for this cell alone.
    #[allow(
        dead_code,
        reason = "declared by the registry; read once the gate and the doc generator consume cells"
    )]
    pub bench_max_steps: Option<u64>,
    /// Overrides the title-level checkpoint for this cell alone.
    pub checkpoint: Option<CheckpointTrigger>,
}

impl MatrixCell {
    /// The form a refusal names this cell by.
    pub(super) fn label(&self) -> String {
        match &self.game_ver {
            Some(v) => format!("fw {} x {v}", self.fw),
            None => format!("fw {}", self.fw),
        }
    }
}

/// Translate the declared rows, or refuse the declaration.
///
/// An absent or empty `[bench] matrix` declares no cells. A non-empty
/// one names exactly one reference cell.
///
/// # Errors
///
/// [`ManifestError::Parse`] for a row that:
///
/// - names an unusable version key,
/// - states an unknown `expect`,
/// - states a `game_ver` on a firmware-shipped title, or omits one
///   anywhere else,
/// - overrides the checkpoint to one the title-level `[rsx] mirror`
///   makes unreachable,
/// - repeats a cell an earlier row already declared.
///
/// [`ManifestError::Parse`] also covers two whole-matrix faults:
///
/// - a non-empty matrix marks other than one cell `reference = true`,
/// - the reference cell is a probe.
///
/// A per-cell checkpoint override goes through [`parse_checkpoint`],
/// so it raises [`ManifestError::UnknownCheckpointKind`] and
/// [`ManifestError::BadCheckpointPc`] unchanged. Those two name the
/// manifest only.
pub(super) fn build(
    rows: Vec<ManifestMatrixRow>,
    source: &GameSource,
    rsx_mirror: bool,
    origin: &Path,
) -> Result<Vec<MatrixCell>, ManifestError> {
    let firmware_exec = matches!(source, GameSource::FirmwareExec { .. });
    let mut cells: Vec<MatrixCell> = Vec::with_capacity(rows.len());
    let mut seen: BTreeSet<(String, Option<String>)> = BTreeSet::new();
    for row in rows {
        let cell = build_cell(row, firmware_exec, rsx_mirror, origin)?;
        if !seen.insert((cell.fw.clone(), cell.game_ver.clone())) {
            return Err(refusal(
                origin,
                format!(
                    "[[bench.matrix]] declares {} twice; one cell holds one result, so a \
                     repeated cell names two results at one anchor",
                    cell.label()
                ),
            ));
        }
        cells.push(cell);
    }
    check_one_reference(&cells, origin)?;
    Ok(cells)
}

fn check_one_reference(cells: &[MatrixCell], origin: &Path) -> Result<(), ManifestError> {
    if cells.is_empty() {
        return Ok(());
    }
    let marked: Vec<&MatrixCell> = cells.iter().filter(|c| c.reference).collect();
    match marked.as_slice() {
        [one] => reference_is_not_a_probe(one, origin),
        [] => Err(refusal(
            origin,
            format!(
                "[[bench.matrix]] declares {} cell(s) ({}) and marks none `reference = true`; \
                 mark the one cell the headline row renders",
                cells.len(),
                render_cells(cells)
            ),
        )),
        several => Err(refusal(
            origin,
            format!(
                "[[bench.matrix]] marks {} cells `reference = true` ({}); exactly one cell is \
                 the headline row",
                several.len(),
                render_refs(several)
            ),
        )),
    }
}

fn reference_is_not_a_probe(cell: &MatrixCell, origin: &Path) -> Result<(), ManifestError> {
    if cell.expect == CellExpectation::Probe {
        return Err(refusal(
            origin,
            format!(
                "[[bench.matrix]] marks {} `reference = true` with expect = {:?}; the headline \
                 row states whether this title converged, and a probe cell's datum is which \
                 error the guest received instead. Make a frontier cell the reference",
                cell.label(),
                CellExpectation::Probe.label()
            ),
        ));
    }
    Ok(())
}

fn build_cell(
    row: ManifestMatrixRow,
    firmware_exec: bool,
    rsx_mirror: bool,
    origin: &Path,
) -> Result<MatrixCell, ManifestError> {
    VersionKey::new(&row.fw).map_err(|e| refusal(origin, format!("[[bench.matrix]] fw: {e}")))?;
    let game_ver = match (row.game_ver, firmware_exec) {
        (Some(v), true) => {
            return Err(refusal(
                origin,
                format!(
                    "[[bench.matrix]] game_ver {v:?} does not apply to a title shipped inside \
                     the firmware: its version axis is the firmware's, so its matrix is one row \
                     per fw. Drop the key"
                ),
            ))
        }
        (None, true) => None,
        (Some(v), false) => {
            if v != BASE_GAME_VER {
                VersionKey::new(&v).map_err(|e| {
                    refusal(
                        origin,
                        format!(
                            "[[bench.matrix]] game_ver is neither {BASE_GAME_VER:?} nor an \
                             update version key: {e}"
                        ),
                    )
                })?;
            }
            Some(v)
        }
        (None, false) => {
            return Err(refusal(
                origin,
                format!(
                    "[[bench.matrix]] row for fw {:?} states no game_ver; name \
                     {BASE_GAME_VER:?} or an update version key",
                    row.fw
                ),
            ))
        }
    };
    let expect = match row.expect {
        None => CellExpectation::Frontier,
        Some(raw) => CellExpectation::from_label(&raw).ok_or_else(|| {
            refusal(
                origin,
                format!(
                    "[[bench.matrix]] unknown expect {raw:?} (accepted: {})",
                    accepted_expects()
                ),
            )
        })?,
    };
    let checkpoint = row
        .checkpoint
        .as_ref()
        .map(|c| parse_checkpoint(c, origin))
        .transpose()?;
    let cell = MatrixCell {
        fw: row.fw,
        game_ver,
        reference: row.reference,
        expect,
        bench_max_steps: row.bench_max_steps,
        checkpoint,
    };
    // `[rsx] mirror` is title-level, so it holds for every cell.
    if cell
        .checkpoint
        .is_some_and(|cp| mirror_makes_checkpoint_unreachable(rsx_mirror, cp))
    {
        return Err(refusal(
            origin,
            format!(
                "[[bench.matrix]] {} overrides the checkpoint to \"first-rsx-write\", which \
                 `[rsx] mirror = true` makes unreachable: the mirror makes the RSX region \
                 writable, so the put-pointer write that FirstRsxWrite watches for cannot \
                 fault. Drop the override or the mirror.",
                cell.label()
            ),
        ));
    }
    Ok(cell)
}

fn accepted_expects() -> String {
    use strum::VariantArray;
    CellExpectation::VARIANTS
        .iter()
        .map(|v| v.label())
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_cells(cells: &[MatrixCell]) -> String {
    cells
        .iter()
        .map(MatrixCell::label)
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_refs(cells: &[&MatrixCell]) -> String {
    cells
        .iter()
        .map(|c| c.label())
        .collect::<Vec<_>>()
        .join(", ")
}

fn refusal(origin: &Path, message: String) -> ManifestError {
    ManifestError::Parse {
        path: origin.to_path_buf(),
        message,
    }
}

#[cfg(test)]
#[path = "tests/matrix_tests.rs"]
mod tests;
