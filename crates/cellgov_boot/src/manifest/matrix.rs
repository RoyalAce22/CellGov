//! The bench matrix: which `(firmware, game version)` cells a title
//! declares.
//!
//! A title with a PARAM.SFO declares one cell by carrying `system_ver`,
//! its floor: the firmware it shipped against, times its base install.
//! That cell is the reference, and `[[bench.matrix]]` rows add cells
//! beside it or attach an override to it. A firmware-shipped title has
//! no floor, so its rows are its whole declaration.
//!
//! The registry declares every cell. The gate and the generated
//! documents read the declared set; nothing here enumerates the store.

use std::collections::BTreeSet;
use std::path::Path;

use cellgov_install::store::VersionKey;
pub use cellgov_install::store::BASE_GAME_VER;
use cellgov_install::system_ver::firmware_version_key;

use super::checkpoint::CheckpointTrigger;
use super::loader::{mirror_makes_checkpoint_unreachable, parse_checkpoint, ManifestError};
use super::model::GameSource;
use super::schema::ManifestMatrixRow;

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

/// The `(firmware, game version)` pair a result is keyed by.
///
/// The registry's declaration and the anchor tree speak this one key.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellKey {
    /// Firmware version key of a `vfs/firmware/<key>/` entry.
    pub fw: String,
    /// [`BASE_GAME_VER`] or an update version key. `None` only for a
    /// title shipped inside the firmware, whose version axis is the
    /// firmware's.
    pub game_ver: Option<String>,
}

impl CellKey {
    /// The form a refusal and a gate verdict name this cell by.
    pub fn label(&self) -> String {
        match &self.game_ver {
            Some(v) => format!("fw {} x {v}", self.fw),
            None => format!("fw {}", self.fw),
        }
    }
}

/// One declared cell: a title at one firmware and one game version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixCell {
    /// The firmware and game version this cell names.
    pub key: CellKey,
    /// What a run of this cell is held to.
    pub expect: CellExpectation,
    /// Overrides the title-level cap for this cell alone.
    pub bench_max_steps: Option<u64>,
    /// Overrides the title-level checkpoint for this cell alone.
    pub checkpoint: Option<CheckpointTrigger>,
    /// Why this cell has no committed measurement yet, when something
    /// outside the registry stops it. For example:
    ///
    /// - a firmware nobody can obtain,
    /// - a defect that ends the boot before the checkpoint.
    pub pending: Option<String>,
}

impl MatrixCell {
    /// See [`CellKey::label`].
    pub(super) fn label(&self) -> String {
        self.key.label()
    }

    /// Whether the row this cell came from states anything beyond the
    /// key: an override or a reason.
    fn carries_something(&self) -> bool {
        self.bench_max_steps.is_some() || self.checkpoint.is_some() || self.pending.is_some()
    }
}

/// The cell `[title] system_ver` derives: the title's floor times its
/// base install.
pub fn derived_key(system_ver: &str) -> CellKey {
    CellKey {
        fw: system_ver.to_string(),
        game_ver: Some(BASE_GAME_VER.to_string()),
    }
}

/// Translate the declaration into the declared cells, or refuse it.
///
/// The derived cell leads the list so the grid and the coverage count
/// keep declaration order; the rows follow in their own order.
///
/// # Errors
///
/// [`ManifestError::Parse`] for a `[title] system_ver` that:
///
/// - is absent on a title with a PARAM.SFO (an hdd or disc source),
/// - is present on a title without one (a firmware-shipped or
///   manifest-relative source),
/// - uses the `MM.mmmm` spelling of PARAM.SFO for a store firmware
///   version key,
/// - names an unusable version key.
///
/// [`ManifestError::Parse`] for a row that:
///
/// - names an unusable version key,
/// - states an unknown `expect`,
/// - states a `game_ver` on a firmware-shipped title, or omits one
///   anywhere else,
/// - overrides the checkpoint to one the title-level `[rsx] mirror`
///   makes unreachable,
/// - repeats a cell an earlier row already declared,
/// - repeats the derived cell with `expect = "probe"`, or with no
///   override and no `pending`,
/// - states a `pending` reason that is empty, or that carries a `|` or
///   a line break.
///
/// A per-cell checkpoint override goes through [`parse_checkpoint`],
/// so it raises [`ManifestError::UnknownCheckpointKind`] and
/// [`ManifestError::BadCheckpointPc`] unchanged. Those two name the
/// manifest only.
pub(super) fn build(
    rows: Vec<ManifestMatrixRow>,
    system_ver: Option<&str>,
    source: &GameSource,
    rsx_mirror: bool,
    origin: &Path,
) -> Result<Vec<MatrixCell>, ManifestError> {
    let firmware_exec = matches!(source, GameSource::FirmwareExec { .. });
    let mut cells: Vec<MatrixCell> = Vec::with_capacity(rows.len() + 1);
    let mut seen: BTreeSet<CellKey> = BTreeSet::new();
    let derived = derived_cell(system_ver, source, origin)?;
    let mut derived_repeated = false;
    if let Some(cell) = derived {
        seen.insert(cell.key.clone());
        cells.push(cell);
    }
    for row in rows {
        let cell = build_cell(row, firmware_exec, rsx_mirror, origin)?;
        let repeats_derived = system_ver.is_some() && cells[0].key == cell.key;
        if repeats_derived && !derived_repeated {
            attach_to_derived(&mut cells[0], cell, origin)?;
            derived_repeated = true;
            continue;
        }
        if !seen.insert(cell.key.clone()) {
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
    Ok(cells)
}

/// The cell `[title] system_ver` derives, or a refusal when the key does
/// not fit the source kind.
fn derived_cell(
    system_ver: Option<&str>,
    source: &GameSource,
    origin: &Path,
) -> Result<Option<MatrixCell>, ManifestError> {
    match (system_ver, source) {
        (Some(v), GameSource::FirmwareExec { .. }) => Err(refusal(
            origin,
            format!(
                "[title] system_ver {v:?} does not apply to a title shipped inside the \
                 firmware: it has no PARAM.SFO to state a floor, and its version axis is the \
                 firmware's, so every [[bench.matrix]] row declares one cell and none is \
                 derived. Drop the key"
            ),
        )),
        (Some(v), GameSource::ManifestRelative { .. }) => Err(refusal(
            origin,
            format!(
                "[title] system_ver {v:?} does not apply to a title built beside its \
                 manifest: it has no PARAM.SFO to state a floor, so its [[bench.matrix]] \
                 rows declare every cell. Drop the key"
            ),
        )),
        (None, GameSource::FirmwareExec { .. } | GameSource::ManifestRelative { .. }) => Ok(None),
        (None, GameSource::Hdd | GameSource::Disc) => Err(refusal(
            origin,
            "[title] system_ver is required: the PS3_SYSTEM_VER the title's own PARAM.SFO \
             states, as a firmware version key (01.5000 is \"1.50\"). It derives the cell \
             the headline row renders, so nobody chooses that cell"
                .to_string(),
        )),
        (Some(v), GameSource::Hdd | GameSource::Disc) => {
            // PARAM.SFO spells the floor `MM.mmmm`; the store keys the
            // firmware by the version its own tree names
            // (`cellgov_install::system_ver`). The raw spelling is a
            // usable path component, so without this refusal it derives
            // a cell under a firmware directory nothing installs.
            if let Ok(key) = firmware_version_key(v) {
                return Err(refusal(
                    origin,
                    format!(
                        "[title] system_ver {v:?} is spelled the way PARAM.SFO spells \
                         PS3_SYSTEM_VER; the key is the firmware version the store names, \
                         {key:?}. Write system_ver = {key:?}"
                    ),
                ));
            }
            VersionKey::new(v).map_err(|e| refusal(origin, format!("[title] system_ver: {e}")))?;
            Ok(Some(MatrixCell {
                key: derived_key(v),
                expect: CellExpectation::Frontier,
                bench_max_steps: None,
                checkpoint: None,
                pending: None,
            }))
        }
    }
}

/// Move the override and reason of a row that repeats the derived cell
/// onto that cell.
fn attach_to_derived(
    derived: &mut MatrixCell,
    row: MatrixCell,
    origin: &Path,
) -> Result<(), ManifestError> {
    if row.expect == CellExpectation::Probe {
        return Err(refusal(
            origin,
            format!(
                "[[bench.matrix]] repeats {}, the cell [title] system_ver derives, with expect = \
                 {:?}; the headline row states whether this title converged, and a probe cell's \
                 datum is which error the guest received instead. Declare the probe at another \
                 cell",
                row.label(),
                CellExpectation::Probe.label()
            ),
        ));
    }
    if !row.carries_something() {
        return Err(refusal(
            origin,
            format!(
                "[[bench.matrix]] repeats {}, the cell [title] system_ver already declares, and \
                 carries no bench_max_steps, checkpoint or pending; the row adds nothing. Drop \
                 it, or give it the override or reason it exists for",
                row.label()
            ),
        ));
    }
    derived.bench_max_steps = row.bench_max_steps;
    derived.checkpoint = row.checkpoint;
    derived.pending = row.pending;
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
    let pending = match row.pending {
        Some(reason) if reason.trim().is_empty() => {
            return Err(refusal(
                origin,
                "[[bench.matrix]] pending is empty; it states why the cell cannot be \
                 measured yet, and an empty reason names nothing. Drop the key or give \
                 the reason"
                    .to_string(),
            ))
        }
        // The reason renders inside a markdown table cell on the
        // title's generated page. A `|` ends that cell early, and a
        // line ending -- LF, CRLF, or a bare CR -- ends the whole row.
        Some(reason) if reason.contains('|') || reason.contains('\n') || reason.contains('\r') => {
            return Err(refusal(
                origin,
                format!(
                    "[[bench.matrix]] pending {reason:?} contains a `|` or a line break; the \
                     reason is rendered in a markdown table cell, which neither survives"
                ),
            ))
        }
        other => other,
    };
    let cell = MatrixCell {
        key: CellKey {
            fw: row.fw,
            game_ver,
        },
        expect,
        bench_max_steps: row.bench_max_steps,
        checkpoint,
        pending,
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

fn refusal(origin: &Path, message: String) -> ManifestError {
    ManifestError::Parse {
        path: origin.to_path_buf(),
        message,
    }
}

#[cfg(test)]
#[path = "tests/matrix_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/matrix_pending_tests.rs"]
mod pending_tests;
