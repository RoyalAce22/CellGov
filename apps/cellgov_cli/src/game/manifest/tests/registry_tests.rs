//! Title-registry directory scanning, lookup, and duplicate detection.

use super::*;
use crate::game::manifest::test_fixtures::{TmpDir, FIRST_RSX_WRITE_TOML, PROCESS_EXIT_TOML};

#[test]
fn registry_scans_directory_in_sorted_order() {
    let tmp = TmpDir::new("manifest_scan");
    std::fs::write(tmp.path().join("NPAA00002.toml"), FIRST_RSX_WRITE_TOML).unwrap();
    std::fs::write(tmp.path().join("NPAA00001.toml"), PROCESS_EXIT_TOML).unwrap();
    let reg = TitleRegistry::scan_dir(tmp.path()).unwrap();
    let names: Vec<&str> = reg.iter().map(|m| m.short_name.as_str()).collect();
    assert_eq!(names, vec!["proc-exit-fixture", "rsx-write-fixture"]);
    assert!(reg.by_short_name("proc-exit-fixture").is_some());
    assert!(reg.by_content_id("NPAA00002").is_some());
    assert!(reg.by_short_name("unknown").is_none());
}

#[test]
fn registry_rejects_duplicate_short_names() {
    let tmp = TmpDir::new("manifest_dupe_name");
    std::fs::write(tmp.path().join("a.toml"), PROCESS_EXIT_TOML).unwrap();
    let collide = PROCESS_EXIT_TOML.replace("NPAA00001", "NPAA99999");
    std::fs::write(tmp.path().join("b.toml"), &collide).unwrap();
    let err = TitleRegistry::scan_dir(tmp.path()).expect_err("duplicate short name");
    assert!(matches!(err, ManifestError::DuplicateShortName { .. }));
}

#[test]
fn registry_rejects_duplicate_content_ids() {
    let tmp = TmpDir::new("manifest_dupe_cid");
    std::fs::write(tmp.path().join("a.toml"), PROCESS_EXIT_TOML).unwrap();
    let collide = PROCESS_EXIT_TOML.replace(r#""proc-exit-fixture""#, r#""proc-exit-fixture-2""#);
    std::fs::write(tmp.path().join("b.toml"), &collide).unwrap();
    let err = TitleRegistry::scan_dir(tmp.path()).expect_err("duplicate content id");
    assert!(matches!(err, ManifestError::DuplicateContentId { .. }));
}

#[test]
fn registry_scan_of_missing_dir_is_empty() {
    let p = Path::new("/nonexistent/cellgov/test/path/does/not/exist");
    let reg = TitleRegistry::scan_dir(p).unwrap();
    assert!(reg.is_empty());
}

#[test]
fn known_names_csv_empty_registry_is_labelled() {
    let reg = TitleRegistry::default();
    assert_eq!(reg.known_names_csv(), "<none>");
}

#[test]
fn duplicate_detection_flags_byte_identical_files() {
    let tmp = TmpDir::new("manifest_identical_dupes");
    std::fs::write(tmp.path().join("a.toml"), PROCESS_EXIT_TOML).unwrap();
    std::fs::write(tmp.path().join("b.toml"), PROCESS_EXIT_TOML).unwrap();
    let err = TitleRegistry::scan_dir(tmp.path()).expect_err("duplicate");
    match err {
        ManifestError::DuplicateShortName {
            files_identical, ..
        }
        | ManifestError::DuplicateContentId {
            files_identical, ..
        } => assert!(files_identical, "identical files must set the hint"),
        other => panic!("unexpected error variant: {other:?}"),
    }
}
