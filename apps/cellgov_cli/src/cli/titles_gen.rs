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
mod tests {
    use super::*;
    use cellgov_compare::{
        BootOutcome, BootSummary, ByteParity, CheckpointKind, Convergence, ConvergenceFailure,
        CrossRunnerSummary, DivergenceClass, ObservedOutcome, UnclassifiedRun,
    };
    use cellgov_time::Budget;
    use std::collections::BTreeMap;
    use std::collections::BTreeSet;

    #[test]
    fn known_flags_covers_every_find_flag_value_call() {
        let source = include_str!("titles_gen.rs");
        let mut used: BTreeSet<&str> = BTreeSet::new();
        for (idx, _) in source.match_indices("find_flag_value(args, \"") {
            let tail = &source[idx + "find_flag_value(args, \"".len()..];
            if let Some(end) = tail.find('"') {
                used.insert(&tail[..end]);
            }
        }
        let declared: BTreeSet<&str> = KNOWN_FLAGS.iter().copied().collect();
        // A real flag is `--` plus lowercase letters / hyphens only;
        // strip doc-comment placeholders that the grep can hit.
        let used: BTreeSet<&str> = used
            .into_iter()
            .filter(|s| {
                s.starts_with("--")
                    && s[2..].chars().all(|c| c.is_ascii_lowercase() || c == '-')
                    && s.len() > 2
            })
            .collect();
        assert_eq!(
            used, declared,
            "KNOWN_FLAGS ({declared:?}) and find_flag_value call sites ({used:?}) disagree; \
             a flag in find_flag_value but not KNOWN_FLAGS lets typos through, \
             a flag in KNOWN_FLAGS but not in find_flag_value is dead-list drift",
        );
    }

    fn title(content_id: &str, display: &str, year: u16, developer: &str) -> TitleManifest {
        use crate::game::manifest::{CheckpointTrigger, Distribution, GameSource};
        TitleManifest {
            content_id: content_id.to_string(),
            short_name: content_id.to_lowercase(),
            display_name: display.to_string(),
            eboot_candidates: vec!["EBOOT.elf".to_string()],
            year,
            developer: developer.to_string(),
            engine: "test-engine".to_string(),
            distribution: Distribution::PsnHdd,
            rap_filename: None,
            checkpoint: CheckpointTrigger::ProcessExit,
            source: GameSource::Hdd,
            rsx_mirror: false,
            content: None,
            mounts: Vec::new(),
        }
    }

    struct TmpDir(PathBuf);
    impl TmpDir {
        fn new(name: &str) -> Self {
            let p = std::env::temp_dir()
                .join(format!("cellgov_titles_gen_{name}_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
    }

    #[test]
    fn row_with_no_summaries_renders_dashes_for_data_cells() {
        let t = title("NPAA00001", "TestTitle", 2010, "TestStudio");
        let tmp = TmpDir::new("dashes");
        let row = render_row(&t, tmp.path()).unwrap();
        assert!(row.contains("| NPAA00001 |"));
        assert!(row.contains("| TestTitle |"));
        assert!(row.contains("| 2010 |"));
        assert!(row.contains("| TestStudio |"));
        let dash_count = row.matches(" -- ").count();
        assert!(dash_count >= 5, "expected several `--` cells in {row}");
    }

    #[test]
    fn row_with_boot_summary_renders_steps_and_insns() {
        let t = title("NPAA00002", "WithBoot", 2007, "Studio");
        let tmp = TmpDir::new("withboot");
        let path = tmp.path().join("NPAA00002/cellgov/boot_summary.json");
        write_json(
            &path,
            &BootSummary::new(
                CheckpointKind::FirstRsxWrite,
                BootOutcome::RsxWriteCheckpoint,
                14_352_589,
                Budget::new(256),
            )
            .unwrap(),
        );
        let row = render_row(&t, tmp.path()).unwrap();
        assert!(
            row.contains("FirstRsxWrite -> RsxWriteCheckpoint"),
            "expected kind + outcome, got: {row}"
        );
        assert!(
            row.contains("14,352,589"),
            "comma-grouped steps expected, got: {row}"
        );
        assert!(
            row.contains("3,674,262,784"),
            "comma-grouped insns expected, got: {row}"
        );
    }

    #[test]
    fn row_with_converged_summary_renders_yes_plus_byte_parity() {
        let t = title("NPAA00003", "Converged", 2008, "Studio");
        let tmp = TmpDir::new("converged");
        let path = tmp
            .path()
            .join("NPAA00003/cross_runner/cross_runner_summary.json");
        write_json(
            &path,
            &CrossRunnerSummary {
                convergence: Convergence::Yes,
                byte_parity: ByteParity::NonSemantic { bytes: 1 },
                per_class_bytes: BTreeMap::from([(DivergenceClass::ElfHeader, 1)]),
                unclassified_bytes: 0,
                unclassified_runs: Vec::new(),
                lowest_offset_class: None,
            },
        );
        let row = render_row(&t, tmp.path()).unwrap();
        assert!(row.contains("| Yes |"), "missing Yes column in {row}");
        assert!(
            row.contains("| 1 non-semantic |"),
            "missing byte parity column in {row}"
        );
    }

    #[test]
    fn row_with_pending_renders_yes_plus_split_byte_count() {
        let t = title("NPAA00005", "Pending", 2008, "Studio");
        let tmp = TmpDir::new("pending");
        let path = tmp
            .path()
            .join("NPAA00005/cross_runner/cross_runner_summary.json");
        write_json(
            &path,
            &CrossRunnerSummary {
                convergence: Convergence::Yes,
                byte_parity: ByteParity::Pending {
                    non_semantic_bytes: 599,
                    unclassified_bytes: 125,
                },
                per_class_bytes: BTreeMap::from([
                    (DivergenceClass::ElfHeader, 599),
                    (DivergenceClass::Unclassified, 125),
                ]),
                unclassified_bytes: 125,
                unclassified_runs: vec![UnclassifiedRun {
                    region_name: "data".to_string(),
                    offset: 0,
                    length: 125,
                }],
                lowest_offset_class: None,
            },
        );
        let row = render_row(&t, tmp.path()).unwrap();
        assert!(row.contains("| Yes |"));
        assert!(
            row.contains("| 599 non-semantic + 125 pending |"),
            "got: {row}"
        );
    }

    #[test]
    fn row_with_diverged_summary_renders_no_plus_dash_byte_parity() {
        let t = title("NPAA00006", "Diverged", 2008, "Studio");
        let tmp = TmpDir::new("diverged");
        let path = tmp
            .path()
            .join("NPAA00006/cross_runner/cross_runner_summary.json");
        let reason = ConvergenceFailure::OutcomeMismatch {
            cellgov: ObservedOutcome::Fault,
            rpcs3: ObservedOutcome::Completed,
        };
        write_json(
            &path,
            &CrossRunnerSummary {
                convergence: Convergence::No {
                    reason: reason.clone(),
                },
                byte_parity: ByteParity::Diverge { reason },
                per_class_bytes: BTreeMap::new(),
                unclassified_bytes: 0,
                unclassified_runs: Vec::new(),
                lowest_offset_class: None,
            },
        );
        let row = render_row(&t, tmp.path()).unwrap();
        assert!(
            row.contains("| No (outcome: Fault vs Completed) |"),
            "missing convergence reason: {row}"
        );
        assert!(
            row.ends_with("| -- |"),
            "row should end with -- byte parity cell: {row}"
        );
    }

    #[test]
    fn checkpoint_cell_shows_actual_outcome_for_all_kinds() {
        let tmp = TmpDir::new("ckpt");

        let t1 = title("NPAA10001", "PE", 2007, "Studio");
        write_json(
            &tmp.path().join("NPAA10001/cellgov/boot_summary.json"),
            &BootSummary::new(
                CheckpointKind::ProcessExit,
                BootOutcome::MaxSteps,
                100,
                Budget::new(256),
            )
            .unwrap(),
        );
        let row = render_row(&t1, tmp.path()).unwrap();
        assert!(row.contains("ProcessExit -> MaxSteps"), "got: {row}");

        let t2 = title("NPAA10002", "Rsx", 2008, "Studio");
        write_json(
            &tmp.path().join("NPAA10002/cellgov/boot_summary.json"),
            &BootSummary::new(
                CheckpointKind::FirstRsxWrite,
                BootOutcome::Fault,
                12,
                Budget::new(256),
            )
            .unwrap(),
        );
        let row = render_row(&t2, tmp.path()).unwrap();
        assert!(row.contains("FirstRsxWrite -> Fault"), "got: {row}");

        let t3 = title("NPAA10003", "Frontier", 2010, "Studio");
        write_json(
            &tmp.path().join("NPAA10003/cellgov/boot_summary.json"),
            &BootSummary::new(
                CheckpointKind::Pc {
                    addr: cellgov_mem::GuestAddr::new(0x1038_1ce8),
                },
                BootOutcome::PcReached(0x1038_1ce8),
                500,
                Budget::new(256),
            )
            .unwrap(),
        );
        let row = render_row(&t3, tmp.path()).unwrap();
        assert!(
            row.contains("Pc=0x10381ce8 -> PcReached(0x10381ce8)"),
            "got: {row}"
        );
    }

    #[test]
    fn corrupt_boot_summary_surfaces_typed_error() {
        let t = title("NPAA20001", "Corrupt", 2008, "Studio");
        let tmp = TmpDir::new("corruptboot");
        let path = tmp.path().join("NPAA20001/cellgov/boot_summary.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        match render_row(&t, tmp.path()) {
            Err(SummaryLoadError::Parse { path: p, .. }) => {
                assert!(p.ends_with("boot_summary.json"), "got path: {p:?}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn corrupt_cross_runner_summary_surfaces_typed_error() {
        let t = title("NPAA20002", "CorruptCross", 2008, "Studio");
        let tmp = TmpDir::new("corruptcross");
        let path = tmp
            .path()
            .join("NPAA20002/cross_runner/cross_runner_summary.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{also not json").unwrap();
        match render_row(&t, tmp.path()) {
            Err(SummaryLoadError::Parse { path: p, .. }) => {
                assert!(p.ends_with("cross_runner_summary.json"), "got path: {p:?}");
            }
            other => panic!("expected Parse error, got {other:?}"),
        }
    }

    #[test]
    fn render_row_is_byte_identical_across_two_invocations() {
        let t = title("NPAA30001", "Deterministic", 2008, "Studio");
        let tmp = TmpDir::new("det");
        write_json(
            &tmp.path().join("NPAA30001/cellgov/boot_summary.json"),
            &BootSummary::new(
                CheckpointKind::FirstRsxWrite,
                BootOutcome::RsxWriteCheckpoint,
                14_352_589,
                Budget::new(256),
            )
            .unwrap(),
        );
        write_json(
            &tmp.path()
                .join("NPAA30001/cross_runner/cross_runner_summary.json"),
            &CrossRunnerSummary {
                convergence: Convergence::Yes,
                byte_parity: ByteParity::NonSemantic { bytes: 1 },
                per_class_bytes: BTreeMap::from([(DivergenceClass::ElfHeader, 1)]),
                unclassified_bytes: 0,
                unclassified_runs: Vec::new(),
                lowest_offset_class: None,
            },
        );
        let a = render_row(&t, tmp.path()).unwrap();
        let b = render_row(&t, tmp.path()).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn boot_present_cross_absent_renders_data_then_dashes() {
        let t = title("NPAA40001", "BootOnly", 2007, "Studio");
        let tmp = TmpDir::new("bootonly");
        write_json(
            &tmp.path().join("NPAA40001/cellgov/boot_summary.json"),
            &BootSummary::new(
                CheckpointKind::FirstRsxWrite,
                BootOutcome::RsxWriteCheckpoint,
                45_697,
                Budget::new(256),
            )
            .unwrap(),
        );
        let row = render_row(&t, tmp.path()).unwrap();
        assert!(row.contains("FirstRsxWrite -> RsxWriteCheckpoint"));
        assert!(row.contains("45,697"));
        assert!(
            row.ends_with("| -- | -- |"),
            "convergence + byte parity should be `--` when no cross-runner: {row}"
        );
    }

    #[test]
    fn cross_present_boot_absent_renders_dashes_then_yes() {
        let t = title("NPAA40002", "CrossOnly", 2008, "Studio");
        let tmp = TmpDir::new("crossonly");
        write_json(
            &tmp.path()
                .join("NPAA40002/cross_runner/cross_runner_summary.json"),
            &CrossRunnerSummary {
                convergence: Convergence::Yes,
                byte_parity: ByteParity::Equivalent,
                per_class_bytes: BTreeMap::new(),
                unclassified_bytes: 0,
                unclassified_runs: Vec::new(),
                lowest_offset_class: None,
            },
        );
        let row = render_row(&t, tmp.path()).unwrap();
        assert!(
            row.contains("| -- | -- | -- |"),
            "boot cells must be `--`: {row}"
        );
        assert!(row.contains("| Yes |"));
        assert!(row.contains("| equivalent |"));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "markdown-table-breaking")]
    fn assert_table_safe_panics_on_pipe_char() {
        assert_table_safe("display_name", "Bad | Name");
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "markdown-table-breaking")]
    fn assert_table_safe_panics_on_newline() {
        assert_table_safe("developer", "Studio\nLine 2");
    }

    #[test]
    fn render_rows_sorted_orders_by_content_id_regardless_of_input_order() {
        let t1 = title("NPAA50003", "Three", 2008, "Studio");
        let t2 = title("NPAA50001", "One", 2008, "Studio");
        let t3 = title("NPAA50002", "Two", 2008, "Studio");
        let tmp = TmpDir::new("sortbyid");
        let rows = render_rows_sorted([&t1, &t2, &t3], tmp.path()).unwrap();
        assert!(rows[0].contains("| NPAA50001 |"), "row 0: {}", rows[0]);
        assert!(rows[1].contains("| NPAA50002 |"), "row 1: {}", rows[1]);
        assert!(rows[2].contains("| NPAA50003 |"), "row 2: {}", rows[2]);
    }

    #[test]
    fn render_rows_sorted_with_empty_input_returns_empty_vec() {
        let tmp = TmpDir::new("empty");
        let rows = render_rows_sorted(std::iter::empty(), tmp.path()).unwrap();
        assert!(rows.is_empty());
    }
}
