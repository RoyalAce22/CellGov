//! `cellgov_cli titles-gen` -- regenerate `docs/titles.md` from
//! `TitleRegistry::scan_dir` + per-title `boot_summary.json` and
//! `cross_runner_summary.json`. ENOENT on a summary renders as `--`;
//! any other I/O or parse failure surfaces as a typed error so a
//! corrupted file cannot read the same as an absent one.

use std::io;
use std::path::{Path, PathBuf};

use cellgov_compare::{format_with_commas, BootSummary, CrossRunnerSummary};

use super::args::find_flag_value;
use super::exit::die;
use crate::game::manifest::{TitleManifest, TitleRegistry};

const TITLES_TEMPLATE: &str = include_str!("templates/titles.md.template");

const DEFAULT_REGISTRY_DIR: &str = "docs/title_manifests";
const DEFAULT_FIXTURES_DIR: &str = "tests/fixtures";
const DEFAULT_OUTPUT: &str = "docs/titles.md";

const KNOWN_FLAGS: &[&str] = &["--registry", "--fixtures-dir", "--output"];

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
}

pub(crate) fn run(args: &[String]) {
    reject_unknown_flags(args);

    let registry_dir =
        find_flag_value(args, "--registry").unwrap_or_else(|| DEFAULT_REGISTRY_DIR.to_string());
    let fixtures_dir =
        find_flag_value(args, "--fixtures-dir").unwrap_or_else(|| DEFAULT_FIXTURES_DIR.to_string());
    let output = find_flag_value(args, "--output").unwrap_or_else(|| DEFAULT_OUTPUT.to_string());

    let registry = TitleRegistry::scan_dir(Path::new(&registry_dir))
        .unwrap_or_else(|e| die(&format!("titles-gen: scan {registry_dir}: {e}")));

    let fixtures = Path::new(&fixtures_dir);
    let rows = render_rows_sorted(registry.iter(), fixtures)
        .unwrap_or_else(|e| die(&format!("titles-gen: {e}")));
    let body =
        super::fixture_gen::apply_subs(TITLES_TEMPLATE, &[("matrix_rows", &rows.join("\n"))]);
    std::fs::write(Path::new(&output), body)
        .unwrap_or_else(|e| die(&format!("titles-gen: write {output}: {e}")));
    println!("titles-gen: wrote {output} ({} title(s))", rows.len());
}

/// Refuse any `--flag` (or `--flag=value`) not in [`KNOWN_FLAGS`]
/// so a typo cannot silently fall back to a default.
fn reject_unknown_flags(args: &[String]) {
    for arg in args {
        let name = arg.split('=').next().unwrap_or(arg);
        if name.starts_with("--") && !KNOWN_FLAGS.contains(&name) {
            die(&format!("titles-gen: unknown flag `{arg}`"));
        }
    }
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
/// `SummaryLoadError` if a summary file exists but cannot be read
/// or parsed. ENOENT renders as `--` cells, not an error.
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

fn load_boot_summary(
    title: &TitleManifest,
    fixtures: &Path,
) -> Result<Option<BootSummary>, SummaryLoadError> {
    let path: PathBuf = fixtures
        .join(&title.content_id)
        .join("cellgov")
        .join("boot_summary.json");
    load_summary_file(&path)
}

fn load_cross_runner_summary(
    title: &TitleManifest,
    fixtures: &Path,
) -> Result<Option<CrossRunnerSummary>, SummaryLoadError> {
    let path: PathBuf = fixtures
        .join(&title.content_id)
        .join("cross_runner")
        .join("cross_runner_summary.json");
    load_summary_file(&path)
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
