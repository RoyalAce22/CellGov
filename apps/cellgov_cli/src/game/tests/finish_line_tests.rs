//! Where the bar's finish line comes from, and when there is none.

use super::*;
use crate::paths::boot_anchor_path;
use cellgov_compare::BootSummary;

fn cell() -> CellKey {
    CellKey {
        fw: "4.93".to_string(),
        game_ver: Some("base".to_string()),
    }
}

/// A fixture tree holding one anchor for `CG_TEST` in [`cell`], in
/// the committed wire shape.
fn root_with_anchor(text: &str) -> cellgov_testkit::scratch::ScratchDir {
    let root = cellgov_testkit::scratch::scratch_labeled("finish_line");
    let path = boot_anchor_path(&root, "CG_TEST", &cell());
    std::fs::create_dir_all(path.parent().expect("anchor dir")).expect("mkdir");
    std::fs::write(path, text).expect("write anchor");
    root
}

const ANCHOR: &str = r#"{
  "checkpoint": { "kind": "process_exit" },
  "outcome": "ProcessExit",
  "steps": 43040,
  "budget": 256
}"#;

#[test]
fn a_recorded_anchor_is_the_finish_line() {
    let root = root_with_anchor(ANCHOR);
    assert_eq!(anchor_steps_under(&root, "CG_TEST", &cell()), Some(43_040));
}

#[test]
fn a_cell_with_no_anchor_has_no_finish_line() {
    let root = root_with_anchor(ANCHOR);
    let other = CellKey {
        fw: "3.55".to_string(),
        game_ver: Some("base".to_string()),
    };
    assert_eq!(anchor_steps_under(&root, "CG_TEST", &other), None);
    assert_eq!(anchor_steps_under(&root, "CG_OTHER", &cell()), None);
}

#[test]
fn an_anchor_that_does_not_parse_is_no_finish_line() {
    let root = root_with_anchor("{ \"steps\": \"forty\" }");
    assert_eq!(anchor_steps_under(&root, "CG_TEST", &cell()), None);
}

/// A directory at the anchor path is unreadable on every platform and
/// absent on none.
#[test]
fn an_unreadable_anchor_is_no_finish_line() {
    let root = cellgov_testkit::scratch::scratch_labeled("finish_line_unreadable");
    let path = boot_anchor_path(&root, "CG_TEST", &cell());
    std::fs::create_dir_all(&path).expect("a directory at the anchor path");
    assert_eq!(anchor_steps_under(&root, "CG_TEST", &cell()), None);
}

#[test]
fn an_unreachable_root_has_no_finish_line() {
    let absent = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("no_such_workspace_root");
    assert_eq!(anchor_steps_under(&absent, "CG_TEST", &cell()), None);
}

#[test]
fn a_run_with_no_cell_or_off_the_anchors_trajectory_has_no_finish_line() {
    assert!(
        anchor_finish_line("BCES00664", Some(&cell()), false).is_some(),
        "the cell's committed anchor is what the two refusals below withhold"
    );
    assert_eq!(anchor_finish_line("BCES00664", None, false), None);
    assert_eq!(anchor_finish_line("BCES00664", Some(&cell()), true), None);
}

#[test]
fn a_committed_cell_anchor_reaches_the_bar_through_the_workspace_root() {
    let path = boot_anchor_path(&workspace_root(), "BCES00664", &cell());
    let recorded: BootSummary =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("committed anchor"))
            .expect("committed anchor parses");
    assert_eq!(
        anchor_finish_line("BCES00664", Some(&cell()), false),
        Some(recorded.steps)
    );
}

#[test]
fn the_finish_line_is_the_anchor_or_the_cap_whichever_comes_first() {
    assert_eq!(within_cap(Some(43_040), 390_000), Some(43_040));
    assert_eq!(within_cap(Some(43_040), 390), Some(390));
    assert_eq!(within_cap(Some(390), 390), Some(390));
}

#[test]
fn a_spent_cap_or_no_anchor_leaves_nothing_to_count_down_to() {
    assert_eq!(within_cap(Some(43_040), 0), None);
    assert_eq!(within_cap(None, 390_000), None);
    assert_eq!(within_cap(Some(0), 390_000), None);
}
