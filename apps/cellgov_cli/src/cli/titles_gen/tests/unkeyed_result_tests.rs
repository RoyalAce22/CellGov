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
