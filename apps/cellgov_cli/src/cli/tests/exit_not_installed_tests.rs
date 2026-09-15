use std::path::PathBuf;

use super::{is_plain_miss, TitleNotInstalled};
use cellgov_boot::manifest::ResolveEbootError;

fn not_found(
    candidates: &[&str],
    probe_errors: Vec<(PathBuf, std::io::Error)>,
    not_regular: Vec<PathBuf>,
) -> ResolveEbootError {
    ResolveEbootError::NotFound {
        searched: vec![PathBuf::from("USRDIR")],
        candidates: candidates.iter().map(|s| s.to_string()).collect(),
        probe_errors,
        not_regular,
    }
}

#[test]
fn a_miss_of_every_candidate_is_an_absence() {
    assert!(is_plain_miss(&not_found(
        &["EBOOT.BIN", "EBOOT.elf"],
        Vec::new(),
        Vec::new()
    )));
}

#[test]
fn a_name_taken_by_a_non_regular_file_is_not_an_absence() {
    let e = not_found(
        &["EBOOT.BIN"],
        Vec::new(),
        vec![PathBuf::from("USRDIR/EBOOT.BIN")],
    );
    assert!(!is_plain_miss(&e));
}

#[test]
fn a_probe_that_failed_for_a_reason_other_than_not_found_is_not_an_absence() {
    let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    let e = not_found(
        &["EBOOT.BIN"],
        vec![(PathBuf::from("USRDIR/EBOOT.BIN"), denied)],
        Vec::new(),
    );
    assert!(!is_plain_miss(&e));
}

#[test]
fn a_manifest_with_nothing_to_probe_is_not_an_absence() {
    assert!(!is_plain_miss(&not_found(&[], Vec::new(), Vec::new())));
}

#[test]
fn a_misconfigured_vfs_root_is_not_an_absence() {
    let e = ResolveEbootError::MisconfiguredVfsRoot {
        vfs_root: PathBuf::from("/"),
        short_name: "t".to_string(),
    };
    assert!(!is_plain_miss(&e));
}

#[test]
fn each_variant_carries_the_marker_note_and_die_text_the_suites_key_on() {
    let no_dir = TitleNotInstalled::NoContentDirectory {
        title: "t".to_string(),
        source: Box::new(not_found(&["EBOOT.BIN"], Vec::new(), Vec::new())),
    };
    assert_eq!(no_dir.marker_note(), "no content directory");
    assert!(
        no_dir
            .to_string()
            .starts_with("resolve_eboot for title t: "),
        "got {no_dir}"
    );

    let no_candidate = TitleNotInstalled::NoEbootCandidate {
        title: "t".to_string(),
        usrdir: "USRDIR".to_string(),
        attempts: "    EBOOT.BIN: read failed: not found".to_string(),
    };
    assert_eq!(no_candidate.marker_note(), "no eboot candidate present");
    assert_eq!(
        no_candidate.to_string(),
        "every eboot_candidate for title t failed under USRDIR:\n    EBOOT.BIN: read failed: not found"
    );
}
