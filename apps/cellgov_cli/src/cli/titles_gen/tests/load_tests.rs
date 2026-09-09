//! What `load_title` does with:
//!
//! - a corrupt file
//! - a summary whose firmware disagrees with the other runner
//! - a summary whose firmware disagrees with the cell it sits in
//! - a result filed under a cell nobody declared

use cellgov_compare::BootOutcome;

use super::super::test_fixtures::*;
use super::*;

#[test]
fn one_library_on_both_sides_loads_the_verdict() {
    let fixtures = Fixtures::new("fw-matched");
    let t = title("NPAA60001", "Matched", 2008, "Studio");
    fixtures.write_cross(
        "NPAA60001",
        &reference_key(),
        &stamped(converged(0), Some(REFERENCE_FW), Some(REFERENCE_FW)),
    );
    let docs = load_title(&t, fixtures.path()).unwrap();
    assert!(docs.reference().unwrap().artifacts.cross.is_some());
}

#[test]
fn two_libraries_refuse_the_cell_naming_both() {
    let fixtures = Fixtures::new("fw-crossed");
    let t = title("NPAA60002", "Crossed", 2008, "Studio");
    fixtures.write_cross(
        "NPAA60002",
        &reference_key(),
        &stamped(converged(0), Some("4.93"), Some("4.92")),
    );
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::Parse { path, err }) => {
            assert!(path.ends_with(CROSS_RUNNER_SUMMARY_FILE), "{path:?}");
            let rendered = err.to_string();
            assert!(
                rendered.contains("4.93") && rendered.contains("4.92"),
                "{rendered}"
            );
        }
        other => panic!("expected a typed parse error, got {other:?}"),
    }
}

#[test]
fn a_stamped_summary_missing_the_other_runners_version_is_refused() {
    let fixtures = Fixtures::new("fw-halfstamped");
    let t = title("NPAA60003", "HalfStamped", 2008, "Studio");
    fixtures.write_cross(
        "NPAA60003",
        &reference_key(),
        &stamped(converged(0), Some(REFERENCE_FW), None),
    );
    assert!(matches!(
        load_title(&t, fixtures.path()),
        Err(SummaryLoadError::Parse { .. })
    ));
}

#[test]
fn a_summary_measured_elsewhere_does_not_answer_for_the_cell_it_sits_in() {
    let fixtures = Fixtures::new("fw-misfiled");
    let t = title("NPAA60005", "Misfiled", 2008, "Studio");
    fixtures.write_cross(
        "NPAA60005",
        &reference_key(),
        &stamped(converged(0), Some("2.76"), Some("2.76")),
    );
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::CellFirmwareMismatch {
            path,
            cell,
            recorded,
        }) => {
            assert!(path.ends_with(CROSS_RUNNER_SUMMARY_FILE), "{path:?}");
            assert_eq!(cell, REFERENCE_FW);
            assert_eq!(recorded, "2.76");
        }
        other => panic!("expected a mis-filed refusal, got {other:?}"),
    }
}

#[test]
fn an_anchor_measured_elsewhere_does_not_answer_for_the_cell_it_sits_in() {
    let fixtures = Fixtures::new("fw-misfiled-anchor");
    let t = title("NPAA60006", "MisfiledAnchor", 2008, "Studio");
    fixtures.write_anchor(
        "NPAA60006",
        &reference_key(),
        &boot_on(BootOutcome::ProcessExit, 11_212, "2.76"),
    );
    assert!(matches!(
        load_title(&t, fixtures.path()),
        Err(SummaryLoadError::CellFirmwareMismatch { .. })
    ));
}

#[test]
fn a_summary_naming_no_firmware_still_loads() {
    let fixtures = Fixtures::new("fw-unstamped");
    let t = title("NPAA60007", "Unstamped", 2008, "Studio");
    fixtures.write_cross("NPAA60007", &reference_key(), &converged(0));
    assert!(load_title(&t, fixtures.path())
        .unwrap()
        .reference()
        .unwrap()
        .artifacts
        .cross
        .is_some());
}

#[test]
fn corrupt_boot_summary_surfaces_typed_error() {
    let fixtures = Fixtures::new("corruptboot");
    let t = title("NPAA20001", "Corrupt", 2008, "Studio");
    let path = anchor_path(fixtures.path(), "NPAA20001", &reference_key());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{not json").unwrap();
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::Parse { path: p, .. }) => {
            assert!(p.ends_with("boot_summary.json"), "got path: {p:?}");
        }
        other => panic!("expected Parse error, got {other:?}"),
    }
}

#[test]
fn corrupt_cross_runner_summary_surfaces_typed_error() {
    let fixtures = Fixtures::new("corruptcross");
    let t = title("NPAA20002", "CorruptCross", 2008, "Studio");
    let path = cross_path(fixtures.path(), "NPAA20002", &reference_key());
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{also not json").unwrap();
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::Parse { path: p, .. }) => {
            assert!(p.ends_with(CROSS_RUNNER_SUMMARY_FILE), "got path: {p:?}");
        }
        other => panic!("expected Parse error, got {other:?}"),
    }
}

#[test]
fn swapping_two_cells_summaries_is_refused_at_each_of_them() {
    let fixtures = Fixtures::new("fw-swapped");
    let other = cell_key("2.76", Some(BASE));
    let mut t = title("NPAA60008", "Swapped", 2008, "Studio");
    t.matrix.push(matrix_cell(other.clone()));

    // Each cell now holds the summary measured at the other.
    fixtures.write_cross(
        "NPAA60008",
        &reference_key(),
        &stamped(converged(0), Some("2.76"), Some("2.76")),
    );
    fixtures.write_cross(
        "NPAA60008",
        &other,
        &stamped(converged(0), Some(REFERENCE_FW), Some(REFERENCE_FW)),
    );
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::CellFirmwareMismatch { cell, recorded, .. }) => {
            assert_eq!((cell.as_str(), recorded.as_str()), (REFERENCE_FW, "2.76"));
        }
        other => panic!("expected the reference cell's refusal, got {other:?}"),
    }

    // Put the reference cell right. The other cell is still crossed,
    // so the refusal moves to it.
    fixtures.write_cross(
        "NPAA60008",
        &reference_key(),
        &stamped(converged(0), Some(REFERENCE_FW), Some(REFERENCE_FW)),
    );
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::CellFirmwareMismatch { cell, recorded, .. }) => {
            assert_eq!((cell.as_str(), recorded.as_str()), ("2.76", REFERENCE_FW));
        }
        other => panic!("expected the second cell's refusal, got {other:?}"),
    }
}

// -- undeclared results --

#[test]
fn an_anchor_for_an_undeclared_cell_is_refused_naming_it() {
    let fixtures = Fixtures::new("undeclared-anchor");
    let t = title("NPAA70001", "Undeclared", 2008, "Studio");
    fixtures.write_anchor(
        "NPAA70001",
        &cell_key("3.55", Some(BASE)),
        &boot(BootOutcome::Fault, 44),
    );
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::UndeclaredCell {
            cell, content_id, ..
        }) => {
            assert_eq!(cell, "fw 3.55 x base");
            assert_eq!(content_id, "NPAA70001");
        }
        other => panic!("expected an undeclared-cell refusal, got {other:?}"),
    }
}

#[test]
fn a_cross_runner_summary_for_an_undeclared_cell_is_refused() {
    let fixtures = Fixtures::new("undeclared-cross");
    let t = title("NPAA70002", "UndeclaredCross", 2008, "Studio");
    fixtures.write_cross("NPAA70002", &cell_key("1.50", Some(BASE)), &converged(0));
    assert!(matches!(
        load_title(&t, fixtures.path()),
        Err(SummaryLoadError::UndeclaredCell { .. })
    ));
}

#[test]
fn an_undeclared_game_version_under_a_declared_firmware_is_still_refused() {
    let fixtures = Fixtures::new("undeclared-gamever");
    let t = title("NPAA70003", "UndeclaredVer", 2008, "Studio");
    fixtures.write_anchor(
        "NPAA70003",
        &cell_key(REFERENCE_FW, Some("02.51")),
        &boot(BootOutcome::Fault, 44),
    );
    match load_title(&t, fixtures.path()) {
        Err(SummaryLoadError::UndeclaredCell { cell, .. }) => {
            assert_eq!(cell, "fw 4.93 x 02.51");
        }
        other => panic!("expected an undeclared-cell refusal, got {other:?}"),
    }
}

#[test]
fn a_title_declaring_no_cells_refuses_an_anchor_on_disk() {
    let fixtures = Fixtures::new("nocells-anchor");
    let mut t = title("NPAA70004", "NoCells", 2009, "Studio");
    t.matrix.clear();
    fixtures.write_anchor(
        "NPAA70004",
        &reference_key(),
        &boot(BootOutcome::ProcessExit, 2_222_222),
    );
    assert!(matches!(
        load_title(&t, fixtures.path()),
        Err(SummaryLoadError::UndeclaredCell { .. })
    ));
}

#[test]
fn a_declared_cell_with_nothing_recorded_is_not_an_undeclared_result() {
    let fixtures = Fixtures::new("declared-empty");
    let t = title("NPAA70005", "Empty", 2009, "Studio");
    let docs = load_title(&t, fixtures.path()).unwrap();
    assert_eq!(docs.cells.len(), 1);
    assert!(docs.reference().unwrap().artifacts.boot.is_none());
}

#[test]
fn a_firmware_shipped_titles_cell_sits_one_level_up_and_is_accepted() {
    let fixtures = Fixtures::new("firmware-exec");
    let t = firmware_exec_title("VSHTEST", "Firmware Exec", &[REFERENCE_FW]);
    fixtures.write_anchor(
        "VSHTEST",
        &cell_key(REFERENCE_FW, None),
        &boot(BootOutcome::MaxSteps, 389_859),
    );
    let docs = load_title(&t, fixtures.path()).unwrap();
    assert!(
        docs.reference().is_none(),
        "a firmware-shipped title derives no reference"
    );
    assert!(docs.cells[0].artifacts.boot.is_some());
}

#[test]
fn a_directory_that_holds_no_summary_declares_no_cell() {
    let fixtures = Fixtures::new("empty-dirs");
    let t = title("NPAA70006", "EmptyDirs", 2009, "Studio");
    std::fs::create_dir_all(
        fixtures
            .path()
            .join("NPAA70006")
            .join("cellgov")
            .join("anchors")
            .join("fw-3.55")
            .join(BASE),
    )
    .unwrap();
    assert!(load_title(&t, fixtures.path()).is_ok());
}
