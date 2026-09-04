//! Reads a cell's committed artifacts and holds each file against the
//! cell it sits in.
//!
//! ENOENT on a declared cell is an absence: the cell renders as
//! unrecorded. Every other failure is typed, so a corrupt file reads
//! differently from a missing one.
//!
//! A summary file states both halves of the cell it was measured at.
//! The loader refuses a file that:
//!
//! - states a firmware or game version its cell directory does not
//!   name,
//! - sits in a cell the manifest does not declare,
//! - sits at a path that names no cell.

use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};

use cellgov_compare::{
    BootSummary, CrossRunnerSummary, FirmwareIdentity, GameIdentity, RunIdentity,
};

use super::cell::{CellArtifacts, CellResult};
use crate::game::manifest::{CellKey, TitleManifest, BASE_GAME_VER};
use crate::paths::CROSS_RUNNER_SUMMARY_FILE;

/// The anchor file every cell's boot measurement is written to.
const BOOT_SUMMARY_FILE: &str = "boot_summary.json";

/// The `fw-` prefix a cell directory's firmware component carries.
const FW_DIR_PREFIX: &str = "fw-";

/// How a refusal names the game-version half of a firmware-shipped
/// title's cell, which has none.
const NO_GAME_VERSION: &str = "(none, a title shipped inside the firmware)";

/// Why a load of a title's committed artifacts failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SummaryLoadError {
    #[error("read {}: {err}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        err: io::Error,
    },
    #[error("parse {}: {err}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        err: serde_json::Error,
    },
    #[error("list {}: {err}", path.display())]
    ScanDir {
        path: PathBuf,
        #[source]
        err: io::Error,
    },
    /// The file states one firmware and the cell it sits in names
    /// another.
    #[error(
        "{}: states firmware {recorded}, but it is filed under the cell for firmware {cell}. \
         A row states the result for the cell it names, so a file measured elsewhere is not \
         an answer for this one",
        path.display()
    )]
    CellFirmwareMismatch {
        path: PathBuf,
        /// Firmware version the cell names.
        cell: String,
        /// Firmware version the file states.
        recorded: String,
    },
    /// The file states one game version and the cell it sits in names
    /// another.
    #[error(
        "{}: states game version {recorded}, but it is filed under the cell for game version \
         {cell}. A row states the result for the cell it names, so a file measured against \
         another version of the title is not an answer for this one",
        path.display()
    )]
    CellGameVersionMismatch {
        path: PathBuf,
        /// Game version the cell names, in the identity's spelling.
        cell: String,
        /// Game version the file states.
        recorded: String,
    },
    /// A result sits at a path no cell key names.
    #[error(
        "{}: sits where no cell of {content_id} names it. A cell's directory is \
         `fw-<version>/<game-version>/`, or `fw-<version>/` for a title shipped inside the \
         firmware, so a result outside that shape is one no row would ever render",
        path.display()
    )]
    UnkeyedResult {
        path: PathBuf,
        /// The title whose tree holds it.
        content_id: String,
    },
    /// A result exists for a cell `[[bench.matrix]]` does not declare.
    #[error(
        "{}: holds a result for {cell}, which {content_id} does not declare. The matrix renders \
         what the registry declares, so an undeclared result is either a cell to declare or a \
         directory to delete",
        path.display()
    )]
    UndeclaredCell {
        path: PathBuf,
        /// The cell the directory names.
        cell: String,
        /// The title whose manifest does not declare it.
        content_id: String,
    },
}

/// One title's whole rendered state, read once and used by both
/// documents.
///
/// `cells` follows the manifest's declaration order, so the grid and
/// the coverage count read the same set the registry declares.
#[derive(Debug)]
pub(crate) struct TitleDocs<'a> {
    pub(crate) title: &'a TitleManifest,
    pub(crate) cells: Vec<(CellKey, CellResult)>,
    /// The reference cell's own artifacts, which the headline row
    /// renders in full. Empty when the title declares no reference.
    pub(crate) reference: CellArtifacts,
}

/// Read every declared cell of one title, after refusing any result
/// filed under a cell it does not declare.
///
/// # Errors
///
/// [`SummaryLoadError`] from [`refuse_undeclared_cells`] or
/// [`load_cell`].
pub(crate) fn load_title<'a>(
    title: &'a TitleManifest,
    fixtures: &Path,
) -> Result<TitleDocs<'a>, SummaryLoadError> {
    refuse_undeclared_cells(title, fixtures)?;
    let reference_key = title.reference_cell().map(|c| c.key.clone());
    let mut cells = Vec::with_capacity(title.matrix.len());
    let mut reference = CellArtifacts::default();
    for cell in &title.matrix {
        let artifacts = load_cell(title, fixtures, &cell.key)?;
        cells.push((cell.key.clone(), CellResult::classify(cell, &artifacts)));
        if reference_key.as_ref() == Some(&cell.key) {
            reference = artifacts;
        }
    }
    Ok(TitleDocs {
        title,
        cells,
        reference,
    })
}

/// Read one declared cell's anchor and cross-runner summary.
///
/// # Errors
///
/// [`SummaryLoadError`] when a file exists but cannot be read, cannot
/// be parsed, or states a firmware or game version other than the one
/// its cell names.
fn load_cell(
    title: &TitleManifest,
    fixtures: &Path,
    cell: &CellKey,
) -> Result<CellArtifacts, SummaryLoadError> {
    let boot_path = crate::paths::boot_anchor_path_in(fixtures, &title.content_id, cell);
    let boot = load_summary_file::<BootSummary>(&boot_path)?;
    if let Some(b) = &boot {
        check_cell(&boot_path, cell, &b.identity)?;
    }

    let cross_path = crate::paths::cross_runner_summary_path_in(fixtures, &title.content_id, cell);
    let cross = load_summary_file::<CrossRunnerSummary>(&cross_path)?;
    if let Some(c) = &cross {
        check_cell(&cross_path, cell, &c.identity)?;
    }

    Ok(CellArtifacts { boot, cross })
}

/// Refuse any committed result filed under a cell the manifest does not
/// declare.
///
/// The scan covers both artifact trees. A tree that does not exist
/// holds no results, so a title with nothing recorded loads clean.
///
/// # Errors
///
/// [`SummaryLoadError::UndeclaredCell`] naming the first extra cell in
/// path order, [`SummaryLoadError::UnkeyedResult`] for a result at a
/// path no cell names, or [`SummaryLoadError::ScanDir`] when a
/// directory cannot be listed.
fn refuse_undeclared_cells(title: &TitleManifest, fixtures: &Path) -> Result<(), SummaryLoadError> {
    let declared: BTreeSet<&CellKey> = title.matrix.iter().map(|c| &c.key).collect();
    let anchors = fixtures
        .join(&title.content_id)
        .join("cellgov")
        .join("anchors");
    let cross = fixtures.join(&title.content_id).join("cross_runner");
    for (root, file) in [
        (anchors, BOOT_SUMMARY_FILE),
        (cross, CROSS_RUNNER_SUMMARY_FILE),
    ] {
        for (cell, path) in recorded_cells(&root, file, &title.content_id)? {
            if !declared.contains(&cell) {
                return Err(SummaryLoadError::UndeclaredCell {
                    path,
                    cell: cell.label(),
                    content_id: title.content_id.clone(),
                });
            }
        }
    }
    Ok(())
}

/// Every cell `root` holds `file` for, with the file's path, in
/// directory-name order.
///
/// A cell's directory is `fw-<ver>/<game-ver>/`, or `fw-<ver>/` for a
/// title shipped inside the firmware. The tree states which shape a
/// given `fw-<ver>` carries: a `file` directly inside it names a
/// firmware-shipped cell, and its subdirectories name game versions.
///
/// The walk refuses a `file` it cannot key:
///
/// - at the root,
/// - under a directory carrying no `fw-` prefix,
/// - under a directory whose name is not UTF-8.
///
/// # Errors
///
/// [`SummaryLoadError::UnkeyedResult`] for such a file, or
/// [`SummaryLoadError::ScanDir`] when a directory cannot be listed.
fn recorded_cells(
    root: &Path,
    file: &str,
    content_id: &str,
) -> Result<Vec<(CellKey, PathBuf)>, SummaryLoadError> {
    let mut found = Vec::new();
    // The layout before cells put the summary here.
    let at_root = root.join(file);
    if at_root.is_file() {
        return Err(unkeyed(at_root, content_id));
    }
    for fw_dir in subdirectories(root)? {
        let Some(fw) = dir_name(&fw_dir).and_then(|n| n.strip_prefix(FW_DIR_PREFIX)) else {
            refuse_results_under(&fw_dir, file, content_id)?;
            continue;
        };
        let fw = fw.to_string();
        let summary = fw_dir.join(file);
        if summary.is_file() {
            found.push((
                CellKey {
                    fw: fw.clone(),
                    game_ver: None,
                },
                summary,
            ));
        }
        for game_dir in subdirectories(&fw_dir)? {
            let summary = game_dir.join(file);
            if !summary.is_file() {
                continue;
            }
            let Some(game_ver) = dir_name(&game_dir) else {
                return Err(unkeyed(summary, content_id));
            };
            found.push((
                CellKey {
                    fw: fw.clone(),
                    game_ver: Some(game_ver.to_string()),
                },
                summary,
            ));
        }
    }
    Ok(found)
}

/// Refuse a `file` under a directory naming no cell, at either depth a
/// cell directory reaches.
fn refuse_results_under(dir: &Path, file: &str, content_id: &str) -> Result<(), SummaryLoadError> {
    let here = dir.join(file);
    if here.is_file() {
        return Err(unkeyed(here, content_id));
    }
    for child in subdirectories(dir)? {
        let summary = child.join(file);
        if summary.is_file() {
            return Err(unkeyed(summary, content_id));
        }
    }
    Ok(())
}

fn unkeyed(path: PathBuf, content_id: &str) -> SummaryLoadError {
    SummaryLoadError::UnkeyedResult {
        path,
        content_id: content_id.to_string(),
    }
}

/// `dir`'s immediate subdirectories, name-sorted. A directory that does
/// not exist has none.
fn subdirectories(dir: &Path) -> Result<Vec<PathBuf>, SummaryLoadError> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(SummaryLoadError::ScanDir {
                path: dir.to_path_buf(),
                err,
            })
        }
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| SummaryLoadError::ScanDir {
            path: dir.to_path_buf(),
            err,
        })?;
        if entry.path().is_dir() {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

/// A directory's own name, or `None` when it is not valid UTF-8.
fn dir_name(dir: &Path) -> Option<&str> {
    dir.file_name().and_then(|n| n.to_str())
}

/// Hold a committed file against both halves of the cell whose
/// directory it sits in.
///
/// `dev record-anchors` refuses to file a mismatched result; nothing
/// else stops one arriving by hand.
fn check_cell(path: &Path, cell: &CellKey, recorded: &RunIdentity) -> Result<(), SummaryLoadError> {
    check_cell_firmware(path, cell, recorded.firmware.as_ref())?;
    check_cell_game_version(path, cell, recorded.game.as_ref())
}

/// Both firmware spellings are the store key of a `vfs/firmware/<key>/`
/// entry, so they compare directly.
///
/// A file written before the store carried versions names no firmware,
/// and raises no mismatch.
fn check_cell_firmware(
    path: &Path,
    cell: &CellKey,
    recorded: Option<&FirmwareIdentity>,
) -> Result<(), SummaryLoadError> {
    match recorded {
        Some(f) if f.version != cell.fw => Err(SummaryLoadError::CellFirmwareMismatch {
            path: path.to_path_buf(),
            cell: cell.fw.clone(),
            recorded: f.version.clone(),
        }),
        Some(_) | None => Ok(()),
    }
}

/// The two sides spell one value differently: a cell's `game_ver` is
/// the bare key, and the composition records a selected update as
/// `update:<key>`.
///
/// A file that names no store entry makes no claim, so it raises no
/// mismatch. [`check_cell_firmware`] reads absence the same way.
fn check_cell_game_version(
    path: &Path,
    cell: &CellKey,
    recorded: Option<&GameIdentity>,
) -> Result<(), SummaryLoadError> {
    let Some(game) = recorded else {
        return Ok(());
    };
    let want = cell.game_ver.as_deref().map(identity_game_version);
    if want.as_deref() == Some(game.version.as_str()) {
        return Ok(());
    }
    Err(SummaryLoadError::CellGameVersionMismatch {
        path: path.to_path_buf(),
        cell: want.unwrap_or_else(|| NO_GAME_VERSION.to_string()),
        recorded: game.version.clone(),
    })
}

/// The `version` a run's identity carries for a cell's game-version
/// axis.
fn identity_game_version(game_ver: &str) -> String {
    if game_ver == BASE_GAME_VER {
        game_ver.to_string()
    } else {
        format!("update:{game_ver}")
    }
}

/// `Ok(None)` on ENOENT; every other I/O or parse failure returns a
/// typed error.
fn load_summary_file<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, SummaryLoadError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(SummaryLoadError::Io {
                path: path.to_path_buf(),
                err,
            })
        }
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|err| SummaryLoadError::Parse {
            path: path.to_path_buf(),
            err,
        })
}

#[cfg(test)]
#[path = "tests/load_tests.rs"]
mod tests;

#[cfg(test)]
mod cell_game_version_tests {
    use cellgov_compare::BootOutcome;

    use super::super::test_fixtures::*;
    use super::*;

    /// An anchor stamped with both halves of the cell it was measured
    /// at; `game` takes the identity spelling of the game version.
    fn boot_at(fw: &str, game: Option<&str>) -> BootSummary {
        let mut b = boot(BootOutcome::ProcessExit, 1_000);
        b.identity = RunIdentity {
            firmware: Some(firmware(fw)),
            game: game.map(|v| GameIdentity {
                title_id: "NPAA61000".to_string(),
                version: v.to_string(),
                app_ver: "01.00".to_string(),
            }),
        };
        b
    }

    #[test]
    fn an_anchor_measured_against_another_game_version_does_not_answer_for_this_cell() {
        let fixtures = Fixtures::new("gamever-misfiled");
        let t = title("NPAA61000", "Misfiled Version", 2008, "Studio");
        fixtures.write_anchor(
            "NPAA61000",
            &reference_key(),
            &boot_at(REFERENCE_FW, Some("update:02.51")),
        );
        match load_title(&t, fixtures.path()) {
            Err(SummaryLoadError::CellGameVersionMismatch { cell, recorded, .. }) => {
                assert_eq!((cell.as_str(), recorded.as_str()), (BASE, "update:02.51"));
            }
            other => panic!("expected a game-version refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_update_cell_accepts_the_identitys_update_spelling() {
        let fixtures = Fixtures::new("gamever-update");
        let key = cell_key(REFERENCE_FW, Some("02.51"));
        let mut t = title("NPAA61001", "Updated", 2008, "Studio");
        t.matrix = vec![matrix_cell(key.clone(), true)];
        fixtures.write_anchor(
            "NPAA61001",
            &key,
            &boot_at(REFERENCE_FW, Some("update:02.51")),
        );
        assert!(load_title(&t, fixtures.path())
            .unwrap()
            .reference
            .boot
            .is_some());
    }

    #[test]
    fn a_firmware_shipped_cell_refuses_an_anchor_naming_a_store_entry() {
        let fixtures = Fixtures::new("gamever-firmware-exec");
        let key = cell_key(REFERENCE_FW, None);
        let mut t = title("VSHVER", "Firmware Exec", 2006, "Studio");
        t.matrix = vec![matrix_cell(key.clone(), true)];
        fixtures.write_anchor("VSHVER", &key, &boot_at(REFERENCE_FW, Some(BASE)));
        assert!(matches!(
            load_title(&t, fixtures.path()),
            Err(SummaryLoadError::CellGameVersionMismatch { .. })
        ));
    }

    #[test]
    fn an_anchor_naming_no_store_entry_still_loads() {
        let fixtures = Fixtures::new("gamever-unstamped");
        let t = title("NPAA61002", "Unstamped Version", 2008, "Studio");
        fixtures.write_anchor("NPAA61002", &reference_key(), &boot_at(REFERENCE_FW, None));
        assert!(load_title(&t, fixtures.path())
            .unwrap()
            .reference
            .boot
            .is_some());
    }
}

#[cfg(test)]
mod unkeyed_result_tests {
    use cellgov_compare::BootOutcome;

    use super::super::test_fixtures::*;
    use super::*;

    #[test]
    fn a_summary_at_the_artifact_root_names_no_cell_and_is_refused() {
        let fixtures = Fixtures::new("unkeyed-root");
        let t = title("NPAA71001", "FlatResidue", 2008, "Studio");
        write_json(
            &fixtures
                .path()
                .join("NPAA71001")
                .join("cross_runner")
                .join(CROSS_RUNNER_SUMMARY_FILE),
            &converged(0),
        );
        match load_title(&t, fixtures.path()) {
            Err(SummaryLoadError::UnkeyedResult { path, content_id }) => {
                assert!(path.ends_with(CROSS_RUNNER_SUMMARY_FILE), "{path:?}");
                assert_eq!(content_id, "NPAA71001");
            }
            other => panic!("expected an unkeyed-result refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_summary_under_a_directory_carrying_no_fw_prefix_is_refused() {
        let fixtures = Fixtures::new("unkeyed-prefix");
        let t = title("NPAA71002", "MisspeltPrefix", 2008, "Studio");
        write_json(
            &fixtures
                .path()
                .join("NPAA71002")
                .join("cellgov")
                .join("anchors")
                .join("fw4.93")
                .join(BASE)
                .join(BOOT_SUMMARY_FILE),
            &boot(BootOutcome::ProcessExit, 44),
        );
        assert!(matches!(
            load_title(&t, fixtures.path()),
            Err(SummaryLoadError::UnkeyedResult { .. })
        ));
    }

    #[test]
    fn a_directory_naming_no_cell_that_holds_no_summary_is_not_a_refusal() {
        let fixtures = Fixtures::new("unkeyed-empty");
        let t = title("NPAA71003", "EmptyStray", 2008, "Studio");
        std::fs::create_dir_all(
            fixtures
                .path()
                .join("NPAA71003")
                .join("cellgov")
                .join("anchors")
                .join("scratch")
                .join("deeper"),
        )
        .unwrap();
        assert!(load_title(&t, fixtures.path()).is_ok());
    }
}
