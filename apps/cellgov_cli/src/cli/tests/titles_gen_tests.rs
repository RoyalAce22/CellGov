//! Per-title markdown row rendering from fixture summaries.

use super::*;
use cellgov_compare::{
    BootOutcome, BootSummary, ByteParity, CheckpointKind, Convergence, ConvergenceFailure,
    CrossRunnerSummary, DivergenceClass, ObservedOutcome, UnclassifiedRun,
};
use cellgov_time::Budget;
use std::collections::BTreeMap;

/// The cell every synthetic title in this file declares as its
/// reference, and whose anchor the headline row renders.
const REFERENCE_FW: &str = "4.93";
const REFERENCE_GAME_VER: &str = "base";

/// The anchor file `render_row` reads for a title built by [`title`].
fn anchor_path(fixtures: &Path, content_id: &str) -> PathBuf {
    fixtures
        .join(content_id)
        .join("cellgov")
        .join("anchors")
        .join(format!("fw-{REFERENCE_FW}"))
        .join(REFERENCE_GAME_VER)
        .join("boot_summary.json")
}

fn title(content_id: &str, display: &str, year: u16, developer: &str) -> TitleManifest {
    use crate::game::manifest::{
        CellExpectation, CellKey, CheckpointTrigger, Distribution, GameSource, MatrixCell,
    };
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
        bench_max_steps: None,
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: vec![MatrixCell {
            key: CellKey {
                fw: REFERENCE_FW.to_string(),
                game_ver: Some(REFERENCE_GAME_VER.to_string()),
            },
            reference: true,
            expect: CellExpectation::Frontier,
            bench_max_steps: None,
            checkpoint: None,
        }],
    }
}

struct TmpDir(PathBuf);
impl TmpDir {
    fn new(name: &str) -> Self {
        let p =
            std::env::temp_dir().join(format!("cellgov_titles_gen_{name}_{}", std::process::id()));
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
    let path = anchor_path(tmp.path(), "NPAA00002");
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
        &anchor_path(tmp.path(), "NPAA10001"),
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
        &anchor_path(tmp.path(), "NPAA10002"),
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
        &anchor_path(tmp.path(), "NPAA10003"),
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
    let path = anchor_path(tmp.path(), "NPAA20001");
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
        &anchor_path(tmp.path(), "NPAA30001"),
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
        &anchor_path(tmp.path(), "NPAA40001"),
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

// -- committed-doc drift gate --

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/cellgov_cli has a workspace root two levels up")
}

/// Collapse table-cell padding so a markdown formatter's column
/// alignment does not read as drift; content changes still do.
/// Mirrors the same helper in `cellgov_lv2`'s `fidelity_doc` gate.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    for line in text.replace("\r\n", "\n").lines() {
        let line = line.trim_end();
        if line.starts_with('|') && line.ends_with('|') {
            let cells: Vec<String> = line
                .trim_matches('|')
                .split('|')
                .map(|c| {
                    let c = c.trim();
                    // Separator cells carry alignment (`---:`); keep
                    // the colon, collapse only the dash run.
                    let core = c.trim_end_matches(':');
                    if core.len() >= 3 && core.chars().all(|ch| ch == '-') {
                        let suffix = if c.ends_with(':') { ":" } else { "" };
                        format!("---{suffix}")
                    } else {
                        c.to_string()
                    }
                })
                .collect();
            out.push_str(&format!("| {} |\n", cells.join(" | ")));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn render_committed_matrix() -> String {
    let root = repo_root();
    let registry =
        TitleRegistry::scan_dir(&root.join(DEFAULT_TITLE_REGISTRY_DIR)).expect("scan titles");
    let (body, n) =
        render_doc(registry.iter(), &root.join("tests/fixtures")).expect("render titles.md body");
    assert!(n > 0, "registry is empty; the gate would pass vacuously");
    body
}

#[test]
fn committed_titles_doc_matches_generator() {
    let path = repo_root().join("docs/titles.md");
    let committed =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert_eq!(
        normalize(&committed),
        normalize(&render_committed_matrix()),
        "docs/titles.md is stale; regenerate with:\n  \
         cargo run --release -p cellgov_cli -- dev titles-gen"
    );
}

#[test]
fn drift_gate_ignores_table_padding() {
    let padded = "| Serial    | Steps |\n| --------- | ----: |\n| NPUA80001 |    11 |\n";
    let tight = "| Serial | Steps |\n| --- | ---: |\n| NPUA80001 | 11 |\n";
    assert_eq!(normalize(padded), normalize(tight));
}

#[test]
fn drift_gate_still_sees_content_change() {
    let a = "| Serial | Steps |\n| --- | ---: |\n| NPUA80001 | 11,224 |\n";
    let b = "| Serial | Steps |\n| --- | ---: |\n| NPUA80001 | 11,299 |\n";
    assert_ne!(normalize(a), normalize(b));
}

#[test]
fn drift_gate_keeps_column_alignment_distinct() {
    let right = "| Steps |\n| ---: |\n";
    let left = "| Steps |\n| --- |\n";
    assert_ne!(normalize(right), normalize(left));
}

/// The anchor of a declared cell that is not the reference; a title
/// built by [`title`] declares its reference at [`REFERENCE_FW`].
fn other_cell_anchor_path(fixtures: &Path, content_id: &str) -> PathBuf {
    fixtures
        .join(content_id)
        .join("cellgov")
        .join("anchors")
        .join("fw-3.55")
        .join(REFERENCE_GAME_VER)
        .join("boot_summary.json")
}

#[test]
fn a_non_reference_cells_anchor_is_never_the_headline_row() {
    let mut t = title("NPAA50001", "TwoCells", 2009, "Studio");
    t.matrix.push(crate::game::manifest::MatrixCell {
        key: crate::game::manifest::CellKey {
            fw: "3.55".to_string(),
            game_ver: Some(REFERENCE_GAME_VER.to_string()),
        },
        reference: false,
        expect: crate::game::manifest::CellExpectation::Frontier,
        bench_max_steps: None,
        checkpoint: None,
    });
    let tmp = TmpDir::new("twocells");
    write_json(
        &other_cell_anchor_path(tmp.path(), "NPAA50001"),
        &BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::Fault,
            7_777_777,
            Budget::new(256),
        )
        .unwrap(),
    );
    let row = render_row(&t, tmp.path()).unwrap();
    assert!(
        !row.contains("7,777,777"),
        "the non-reference cell's steps reached the row: {row}"
    );

    write_json(
        &anchor_path(tmp.path(), "NPAA50001"),
        &BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::ProcessExit,
            1_111_111,
            Budget::new(256),
        )
        .unwrap(),
    );
    let row = render_row(&t, tmp.path()).unwrap();
    assert!(row.contains("1,111,111"), "{row}");
    assert!(!row.contains("7,777,777"), "{row}");
}

#[test]
fn a_title_declaring_no_cells_renders_dashes_over_an_anchor_on_disk() {
    let mut t = title("NPAA50002", "NoCells", 2009, "Studio");
    t.matrix.clear();
    let tmp = TmpDir::new("nocells");
    write_json(
        &anchor_path(tmp.path(), "NPAA50002"),
        &BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::ProcessExit,
            2_222_222,
            Budget::new(256),
        )
        .unwrap(),
    );
    let row = render_row(&t, tmp.path()).unwrap();
    assert!(!row.contains("2,222,222"), "{row}");
}
