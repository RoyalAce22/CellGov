//! The headline row, the coverage line, and the committed-doc drift
//! gate.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cellgov_compare::{BootOutcome, BootSummary, CheckpointKind};
use cellgov_time::Budget;

use super::super::load::load_title;
use super::super::run::{render_docs, GeneratedDoc};
use super::super::test_fixtures::*;
use super::*;
use crate::cli::title::DEFAULT_TITLE_REGISTRY_DIR;
use crate::game::manifest::TitleRegistry;

fn headline_row(t: &TitleManifest, fixtures: &Path) -> String {
    render_row(&load_title(t, fixtures).unwrap())
}

#[test]
fn row_with_no_summaries_renders_dashes_for_data_cells() {
    let t = title("NPAA00001", "TestTitle", 2010, "TestStudio");
    let tmp = Fixtures::new("dashes");
    let row = headline_row(&t, tmp.path());
    assert!(row.contains("[NPAA00001](titles/NPAA00001.md)"), "{row}");
    assert!(row.contains("| TestTitle |"));
    assert!(row.contains("| 2010 |"));
    assert!(row.contains("| TestStudio |"));
    assert!(
        row.ends_with("| -- | -- | -- | -- | -- |"),
        "every data cell after Config must be `--`: {row}"
    );
}

#[test]
fn the_config_column_names_the_reference_cell() {
    let t = title("NPAA00010", "Configured", 2009, "Studio");
    let tmp = Fixtures::new("config");
    assert!(
        headline_row(&t, tmp.path()).contains("| fw 4.93 x base |"),
        "{}",
        headline_row(&t, tmp.path())
    );
}

#[test]
fn a_title_declaring_no_cells_names_no_configuration() {
    let mut t = title("NPAA00011", "NoCells", 2009, "Studio");
    t.system_ver = None;
    t.matrix.clear();
    let tmp = Fixtures::new("config-none");
    let row = headline_row(&t, tmp.path());
    assert!(!row.contains("fw "), "{row}");
    assert!(
        row.ends_with("| -- | -- | -- | -- | -- | -- |"),
        "the Config cell must be `--` alongside the data cells: {row}"
    );
}

#[test]
fn row_with_boot_summary_renders_steps_and_insns() {
    let t = title("NPAA00002", "WithBoot", 2007, "Studio");
    let tmp = Fixtures::new("withboot");
    tmp.write_anchor(
        "NPAA00002",
        &reference_key(),
        &BootSummary::new(
            CheckpointKind::FirstRsxWrite,
            BootOutcome::RsxWriteCheckpoint,
            14_352_589,
            Budget::new(256),
        )
        .unwrap(),
    );
    let row = headline_row(&t, tmp.path());
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
    let tmp = Fixtures::new("converged");
    tmp.write_cross("NPAA00003", &reference_key(), &converged(1));
    let row = headline_row(&t, tmp.path());
    assert!(row.contains("| Yes |"), "missing Yes column in {row}");
    assert!(
        row.contains("| 1 non-semantic |"),
        "missing byte parity column in {row}"
    );
}

#[test]
fn row_with_pending_renders_yes_plus_split_byte_count() {
    let t = title("NPAA00005", "Pending", 2008, "Studio");
    let tmp = Fixtures::new("pending");
    tmp.write_cross("NPAA00005", &reference_key(), &converged_pending(599, 125));
    let row = headline_row(&t, tmp.path());
    assert!(row.contains("| Yes |"));
    assert!(
        row.contains("| 599 non-semantic + 125 pending |"),
        "got: {row}"
    );
}

#[test]
fn row_with_diverged_summary_renders_no_plus_dash_byte_parity() {
    let t = title("NPAA00006", "Diverged", 2008, "Studio");
    let tmp = Fixtures::new("diverged");
    tmp.write_cross("NPAA00006", &reference_key(), &diverged());
    let row = headline_row(&t, tmp.path());
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
    let tmp = Fixtures::new("ckpt");

    let t1 = title("NPAA10001", "PE", 2007, "Studio");
    tmp.write_anchor(
        "NPAA10001",
        &reference_key(),
        &BootSummary::new(
            CheckpointKind::ProcessExit,
            BootOutcome::MaxSteps,
            100,
            Budget::new(256),
        )
        .unwrap(),
    );
    assert!(
        headline_row(&t1, tmp.path()).contains("ProcessExit -> MaxSteps"),
        "{}",
        headline_row(&t1, tmp.path())
    );

    let t2 = title("NPAA10002", "Rsx", 2008, "Studio");
    tmp.write_anchor(
        "NPAA10002",
        &reference_key(),
        &BootSummary::new(
            CheckpointKind::FirstRsxWrite,
            BootOutcome::Fault,
            12,
            Budget::new(256),
        )
        .unwrap(),
    );
    assert!(
        headline_row(&t2, tmp.path()).contains("FirstRsxWrite -> Fault"),
        "{}",
        headline_row(&t2, tmp.path())
    );

    let t3 = title("NPAA10003", "Frontier", 2010, "Studio");
    tmp.write_anchor(
        "NPAA10003",
        &reference_key(),
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
    assert!(
        headline_row(&t3, tmp.path()).contains("Pc=0x10381ce8 -> PcReached(0x10381ce8)"),
        "{}",
        headline_row(&t3, tmp.path())
    );
}

#[test]
fn render_row_is_byte_identical_across_two_invocations() {
    let t = title("NPAA30001", "Deterministic", 2008, "Studio");
    let tmp = Fixtures::new("det");
    tmp.write_anchor(
        "NPAA30001",
        &reference_key(),
        &BootSummary::new(
            CheckpointKind::FirstRsxWrite,
            BootOutcome::RsxWriteCheckpoint,
            14_352_589,
            Budget::new(256),
        )
        .unwrap(),
    );
    tmp.write_cross("NPAA30001", &reference_key(), &converged(1));
    assert_eq!(headline_row(&t, tmp.path()), headline_row(&t, tmp.path()));
}

#[test]
fn boot_present_cross_absent_renders_data_then_dashes() {
    let t = title("NPAA40001", "BootOnly", 2007, "Studio");
    let tmp = Fixtures::new("bootonly");
    tmp.write_anchor(
        "NPAA40001",
        &reference_key(),
        &BootSummary::new(
            CheckpointKind::FirstRsxWrite,
            BootOutcome::RsxWriteCheckpoint,
            45_697,
            Budget::new(256),
        )
        .unwrap(),
    );
    let row = headline_row(&t, tmp.path());
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
    let tmp = Fixtures::new("crossonly");
    tmp.write_cross("NPAA40002", &reference_key(), &converged(0));
    let row = headline_row(&t, tmp.path());
    assert!(
        row.ends_with("| fw 4.93 x base | -- | -- | -- | Yes | equivalent |"),
        "boot cells must be `--` beside a recorded convergence: {row}"
    );
}

#[test]
#[should_panic(expected = "markdown-table-breaking")]
fn assert_table_safe_panics_on_pipe_char() {
    assert_table_safe("display_name", "Bad | Name");
}

#[test]
#[should_panic(expected = "markdown-table-breaking")]
fn assert_table_safe_panics_on_newline() {
    assert_table_safe("developer", "Studio\nLine 2");
}

#[test]
#[should_panic(expected = "markdown-table-breaking")]
fn assert_table_safe_panics_on_bare_carriage_return() {
    assert_table_safe("engine", "Studio\rLine 2");
}

#[test]
fn sort_by_content_id_orders_regardless_of_input_order() {
    let t1 = title("NPAA50003", "Three", 2008, "Studio");
    let t2 = title("NPAA50001", "One", 2008, "Studio");
    let t3 = title("NPAA50002", "Two", 2008, "Studio");
    let sorted = sort_by_content_id([&t1, &t2, &t3]);
    let ids: Vec<&str> = sorted.iter().map(|t| t.content_id.as_str()).collect();
    assert_eq!(ids, ["NPAA50001", "NPAA50002", "NPAA50003"]);
}

#[test]
fn sort_by_content_id_with_empty_input_returns_empty_vec() {
    assert!(sort_by_content_id(std::iter::empty()).is_empty());
}

// -- coverage --

#[test]
fn coverage_counts_declared_and_recorded_cells_apart() {
    let mut t = title("NPAA60010", "Coverage", 2009, "Studio");
    t.matrix.push(matrix_cell(cell_key("3.55", Some(BASE))));
    t.matrix.push(matrix_cell(cell_key("1.50", Some(BASE))));
    let tmp = Fixtures::new("coverage");
    tmp.write_cross("NPAA60010", &reference_key(), &converged(0));
    tmp.write_anchor(
        "NPAA60010",
        &cell_key("3.55", Some(BASE)),
        &boot(BootOutcome::Fault, 44),
    );
    let docs = load_title(&t, tmp.path()).unwrap();
    assert_eq!(
        render_coverage(&[&docs]),
        "1 game title(s), 3 firmware(s), 3 declared cell(s), 2 recorded."
    );
}

#[test]
fn coverage_counts_a_firmware_once_across_titles_that_share_it() {
    let a = title("NPAA60011", "A", 2009, "Studio");
    let b = title("NPAA60012", "B", 2009, "Studio");
    let tmp = Fixtures::new("coverage-shared");
    let a = load_title(&a, tmp.path()).unwrap();
    let b = load_title(&b, tmp.path()).unwrap();
    assert_eq!(
        render_coverage(&[&a, &b]),
        "2 game title(s), 1 firmware(s), 2 declared cell(s), 0 recorded."
    );
}

#[test]
fn the_index_carries_the_coverage_line() {
    let t = title("NPAA60013", "Indexed", 2009, "Studio");
    let tmp = Fixtures::new("coverage-index");
    let body = render(&[load_title(&t, tmp.path()).unwrap()]);
    assert!(
        body.contains("Coverage: 1 game title(s), 1 firmware(s), 1 declared cell(s), 0 recorded."),
        "{body}"
    );
}

// -- committed-doc drift gate --

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("apps/cellgov_cli has a workspace root two levels up")
}

/// Collapse table-cell padding so a markdown formatter's column
/// alignment does not read as drift. A content change still does.
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

fn render_committed_docs() -> Vec<GeneratedDoc> {
    let root = repo_root();
    let registry =
        TitleRegistry::scan_dir(&root.join(DEFAULT_TITLE_REGISTRY_DIR)).expect("scan titles");
    let docs =
        render_docs(registry.iter(), &root.join("tests/fixtures")).expect("render title documents");
    assert!(
        docs.len() > 2,
        "registry is empty; the gate would pass vacuously"
    );
    docs
}

const REGENERATE: &str = "regenerate with:\n  cargo run --release -p cellgov_cli -- dev titles-gen";

#[test]
fn committed_titles_doc_matches_generator() {
    let root = repo_root().join("docs");
    for doc in render_committed_docs() {
        let path = root.join(&doc.path);
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}; {REGENERATE}", path.display()));
        assert_eq!(
            normalize(&committed),
            normalize(&doc.body),
            "{} is stale; {REGENERATE}",
            path.display()
        );
    }
}

#[test]
fn the_committed_detail_pages_are_exactly_the_generated_set() {
    let root = repo_root().join("docs");
    let generated: BTreeSet<PathBuf> = render_committed_docs()
        .iter()
        .map(|d| root.join(&d.path))
        .collect();
    let committed: BTreeSet<PathBuf> = std::fs::read_dir(root.join("titles"))
        .expect("docs/titles/ exists")
        .map(|e| e.expect("read docs/titles/ entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    let orphans: Vec<&PathBuf> = committed.difference(&generated).collect();
    assert!(
        orphans.is_empty(),
        "no title declares {orphans:?}; {REGENERATE}"
    );
}

#[test]
fn drift_gate_ignores_table_padding() {
    let padded = "| Serial    | Steps |\n| --------- | ----: |\n| NPAA00001 |    11 |\n";
    let tight = "| Serial | Steps |\n| --- | ---: |\n| NPAA00001 | 11 |\n";
    assert_eq!(normalize(padded), normalize(tight));
}

#[test]
fn drift_gate_still_sees_content_change() {
    let a = "| Serial | Steps |\n| --- | ---: |\n| NPAA00001 | 11,224 |\n";
    let b = "| Serial | Steps |\n| --- | ---: |\n| NPAA00001 | 11,299 |\n";
    assert_ne!(normalize(a), normalize(b));
}

#[test]
fn drift_gate_keeps_column_alignment_distinct() {
    let right = "| Steps |\n| ---: |\n";
    let left = "| Steps |\n| --- |\n";
    assert_ne!(normalize(right), normalize(left));
}

#[test]
fn a_firmware_shipped_title_has_no_row_in_the_index_and_is_not_counted() {
    let game = title("NPAA00020", "Game", 2007, "Studio");
    let shipped = firmware_exec_title("VSHIDX", "System Software", &[REFERENCE_FW, "1.50"]);
    let tmp = Fixtures::new("index-no-firmware-exec");
    tmp.write_anchor(
        "VSHIDX",
        &cell_key(REFERENCE_FW, None),
        &boot(BootOutcome::MaxSteps, 389_859),
    );
    let body = render(&[
        load_title(&game, tmp.path()).unwrap(),
        load_title(&shipped, tmp.path()).unwrap(),
    ]);
    assert!(body.contains("[NPAA00020](titles/NPAA00020.md)"), "{body}");
    assert!(!body.contains("VSHIDX"), "{body}");
    assert!(!body.contains("389,859"), "{body}");
    assert!(
        body.contains("Coverage: 1 game title(s), 1 firmware(s), 1 declared cell(s), 0 recorded."),
        "the firmware-shipped title's two cells must not reach the count: {body}"
    );
}

#[test]
fn the_config_column_names_the_floor_whatever_else_is_declared() {
    let mut t = title("NPAA00021", "Floored", 2007, "Studio");
    t.system_ver = Some("1.50".to_string());
    t.matrix = vec![
        matrix_cell(cell_key("1.50", Some(BASE))),
        matrix_cell(reference_key()),
    ];
    let tmp = Fixtures::new("config-floor");
    tmp.write_anchor(
        "NPAA00021",
        &reference_key(),
        &boot(BootOutcome::ProcessExit, 4_444_444),
    );
    let row = headline_row(&t, tmp.path());
    assert!(row.contains("| fw 1.50 x base |"), "{row}");
    assert!(
        !row.contains("4,444,444"),
        "the newer firmware's anchor is not the headline: {row}"
    );
}

#[test]
fn a_non_reference_cells_anchor_is_never_the_headline_row() {
    let mut t = title("NPAA50001", "TwoCells", 2009, "Studio");
    t.matrix.push(matrix_cell(cell_key("3.55", Some(BASE))));
    let tmp = Fixtures::new("twocells");
    tmp.write_anchor(
        "NPAA50001",
        &cell_key("3.55", Some(BASE)),
        &boot(BootOutcome::Fault, 7_777_777),
    );
    let row = headline_row(&t, tmp.path());
    assert!(
        !row.contains("7,777,777"),
        "the non-reference cell's steps reached the row: {row}"
    );

    tmp.write_anchor(
        "NPAA50001",
        &reference_key(),
        &boot(BootOutcome::ProcessExit, 1_111_111),
    );
    let row = headline_row(&t, tmp.path());
    assert!(row.contains("1,111,111"), "{row}");
    assert!(!row.contains("7,777,777"), "{row}");
}
