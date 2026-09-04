//! `cellgov dev titles-gen` -- regenerate `docs/titles.md` from
//! `TitleRegistry::scan_dir`, the reference cell's anchor, and
//! `cross_runner_summary.json`.
//!
//! ENOENT on a summary renders as `--`. Any other I/O or parse failure
//! surfaces as a typed error, so a corrupted file cannot read the same
//! as an absent one. Two firmware disagreements refuse the row the same
//! way:
//!
//! - the two runners of one summary name different libraries,
//! - a summary names a library other than the one its cell names.

use std::io;
use std::path::{Path, PathBuf};

use cellgov_compare::{format_with_commas, BootSummary, CrossRunnerSummary, FirmwareIdentity};

use super::exit::die;
use super::parse::TitlesGenArgs;
use super::title::DEFAULT_TITLE_REGISTRY_DIR;
use crate::game::manifest::{CellKey, TitleManifest, TitleRegistry};

const TITLES_TEMPLATE: &str = include_str!("templates/titles.md.template");

const DEFAULT_OUTPUT: &str = "docs/titles.md";

/// Why loading a per-title summary JSON file failed. ENOENT is
/// `Ok(None)` upstream, never this error.
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
}

pub(crate) fn run(args: &TitlesGenArgs) {
    let registry_dir = args
        .registry
        .clone()
        .unwrap_or_else(|| DEFAULT_TITLE_REGISTRY_DIR.to_string());
    let fixtures_dir = args
        .fixtures_dir
        .clone()
        .unwrap_or_else(|| crate::paths::DEFAULT_FIXTURES_DIR.to_string());
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| DEFAULT_OUTPUT.to_string());

    let registry = TitleRegistry::scan_dir(Path::new(&registry_dir))
        .unwrap_or_else(|e| die(&format!("titles-gen: scan {registry_dir}: {e}")));

    let fixtures = Path::new(&fixtures_dir);
    let (body, n_titles) =
        render_doc(registry.iter(), fixtures).unwrap_or_else(|e| die(&format!("titles-gen: {e}")));
    std::fs::write(Path::new(&output), body)
        .unwrap_or_else(|e| die(&format!("titles-gen: write {output}: {e}")));
    println!("titles-gen: wrote {output} ({n_titles} title(s))");
}

/// Render the whole `docs/titles.md` body, plus the row count.
///
/// Shared by [`run`] and the committed-doc drift gate so the test
/// checks the same bytes the generator writes.
///
/// # Errors
///
/// `SummaryLoadError` if a per-title summary exists but cannot be
/// read or parsed.
fn render_doc<'a>(
    titles: impl IntoIterator<Item = &'a TitleManifest>,
    fixtures: &Path,
) -> Result<(String, usize), SummaryLoadError> {
    let rows = render_rows_sorted(titles, fixtures)?;
    let body =
        super::fixture_gen::apply_subs(TITLES_TEMPLATE, &[("matrix_rows", &rows.join("\n"))]);
    Ok((body, rows.len()))
}

/// Render every title's row in `content_id`-ascending order.
fn render_rows_sorted<'a>(
    titles: impl IntoIterator<Item = &'a TitleManifest>,
    fixtures: &Path,
) -> Result<Vec<String>, SummaryLoadError> {
    let mut titles: Vec<&TitleManifest> = titles.into_iter().collect();
    titles.sort_by(|a, b| a.content_id.cmp(&b.content_id));
    debug_assert!(
        titles.windows(2).all(|w| w[0].content_id < w[1].content_id),
        "titles-gen: duplicate content_id in registry"
    );
    titles.iter().map(|t| render_row(t, fixtures)).collect()
}

/// One markdown table row.
///
/// # Errors
///
/// `SummaryLoadError` if a summary file exists but cannot be read,
/// cannot be parsed, or states a firmware other than the one its cell
/// names. ENOENT renders as `--` cells, not an error.
fn render_row(title: &TitleManifest, fixtures: &Path) -> Result<String, SummaryLoadError> {
    let boot = load_boot_summary(title, fixtures)?;
    let cross = load_cross_runner_summary(title, fixtures)?;

    let (checkpoint_cell, steps_cell, insns_cell) = match &boot {
        Some(b) => (
            format_checkpoint(b),
            format_with_commas(b.steps),
            format_with_commas(b.insns()),
        ),
        None => ("--".to_string(), "--".to_string(), "--".to_string()),
    };
    let (convergence_cell, byte_parity_cell) = match &cross {
        Some(c) => c.display_matrix_columns(),
        None => ("--".to_string(), "--".to_string()),
    };

    assert_table_safe("content_id", &title.content_id);
    assert_table_safe("display_name", &title.display_name);
    assert_table_safe("developer", &title.developer);
    assert_table_safe("engine", &title.engine);

    Ok(format!(
        "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
        title.content_id,
        title.display_name,
        title.year,
        title.developer,
        title.engine,
        title.distribution.format_label(),
        checkpoint_cell,
        steps_cell,
        insns_cell,
        convergence_cell,
        byte_parity_cell,
    ))
}

/// `<checkpoint kind> -> <observed outcome>`; both columns render so
/// a regressed run (`FirstRsxWrite -> Fault`) is visibly distinct
/// from a clean one (`FirstRsxWrite -> RsxWriteCheckpoint`).
fn format_checkpoint(b: &BootSummary) -> String {
    format!("{} -> {}", b.checkpoint.as_markdown_label(), b.outcome)
}

/// Debug-only check that `value` contains no markdown-table-breaking
/// `|` or newline.
fn assert_table_safe(field: &str, value: &str) {
    debug_assert!(
        !value.contains('|') && !value.contains('\n'),
        "title manifest field `{field}` contains markdown-table-breaking char(s): {value:?}"
    );
}

/// The headline row's measurement: the anchor of the cell the manifest
/// marks `reference = true`.
///
/// A title that declares no reference cell names no configuration for
/// the row, so the row's data cells render as `--`.
fn load_boot_summary(
    title: &TitleManifest,
    fixtures: &Path,
) -> Result<Option<BootSummary>, SummaryLoadError> {
    let Some(cell) = title.reference_cell() else {
        return Ok(None);
    };
    let path = crate::paths::boot_anchor_path_in(fixtures, &title.content_id, &cell.key);
    let Some(summary) = load_summary_file::<BootSummary>(&path)? else {
        return Ok(None);
    };
    check_cell_firmware(&path, &cell.key, summary.identity.firmware.as_ref())?;
    Ok(Some(summary))
}

/// The headline row's cross-runner verdict, from the reference cell
/// [`load_boot_summary`] also reads.
fn load_cross_runner_summary(
    title: &TitleManifest,
    fixtures: &Path,
) -> Result<Option<CrossRunnerSummary>, SummaryLoadError> {
    let Some(cell) = title.reference_cell() else {
        return Ok(None);
    };
    let path = crate::paths::cross_runner_summary_path_in(fixtures, &title.content_id, &cell.key);
    let Some(summary) = load_summary_file::<CrossRunnerSummary>(&path)? else {
        return Ok(None);
    };
    check_cell_firmware(&path, &cell.key, summary.identity.firmware.as_ref())?;
    Ok(Some(summary))
}

/// Hold a committed file against the cell whose directory it sits in.
///
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

/// `Ok(None)` on ENOENT; any other I/O or parse failure surfaces
/// as a typed error so a corrupted file cannot read the same as
/// an absent one.
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
#[path = "tests/titles_gen_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/titles_gen_firmware_tests.rs"]
mod firmware_tests;
