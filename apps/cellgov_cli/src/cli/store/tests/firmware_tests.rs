#![cfg(feature = "decrypt")]

use std::path::PathBuf;

use super::*;

/// A PUP carries 20-odd packages, and almost none of them prune or skip
/// anything. A line that printed the zeros on every package would bury
/// the ones that did something.
#[test]
fn a_package_summary_names_only_the_counts_it_has() {
    let mut p = PackageSummary {
        package: "dev_flash_013.tar".to_string(),
        written: 104,
        pruned: 0,
        skipped: 0,
    };
    assert_eq!(package_summary_line(&p), "dev_flash_013.tar: 104 files");

    p.pruned = 75;
    assert_eq!(
        package_summary_line(&p),
        "dev_flash_013.tar: 104 files, 75 pruned"
    );

    p.skipped = 2;
    assert_eq!(
        package_summary_line(&p),
        "dev_flash_013.tar: 104 files, 75 pruned, 2 entries addressing no file"
    );
}

/// A package that legitimately carries no dev_flash content is a real
/// case: `dev_flash_000` of retail 4.93 extracts zero files.
#[test]
fn a_package_that_extracted_nothing_still_reports_its_zero() {
    let p = PackageSummary {
        package: "dev_flash_000.tar".to_string(),
        written: 0,
        pruned: 0,
        skipped: 0,
    };
    assert_eq!(package_summary_line(&p), "dev_flash_000.tar: 0 files");
}

#[test]
fn a_single_omission_does_not_read_as_a_plural() {
    assert_eq!(plural(1, "file", "files"), "1 file");
    assert_eq!(plural(0, "file", "files"), "0 files");
    assert_eq!(plural(2, "file", "files"), "2 files");
}

fn partial_install() -> FirmwareInstallError {
    use cellgov_install::firmware_install::PackageFailure;
    use cellgov_install::tar::{ExtractError, TarParseError};

    FirmwareInstallError::PartialInstall {
        files: 3,
        packages: 2,
        packages_failed: vec![PackageFailure::InnerTar {
            package: "dev_flash_010.tar".to_string(),
            source: TarParseError::NotUstarHeader { offset: 0x200 },
        }],
        extract_errors: vec![ExtractError::PathTraversal {
            guest_path: "../escape.self".to_string(),
            host_path: PathBuf::from("escape.self"),
        }],
    }
}

#[test]
fn a_partial_install_names_every_package_and_entry_it_lost() {
    let lines = install_failure_detail(&partial_install());
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].contains("dev_flash_010.tar"), "{lines:?}");
    assert!(lines[1].contains("escape.self"), "{lines:?}");
}

/// `StagingResidue` renders only the wrapped fault's summary counts,
/// and that is exactly when the operator has residue on disk and needs
/// the names.
#[test]
fn staging_residue_does_not_swallow_the_partial_install_it_wraps() {
    let expected = install_failure_detail(&partial_install());
    let wrapped = FirmwareInstallError::StagingResidue {
        path: PathBuf::from("vfs/firmware/.firmware-staging"),
        source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        cause: Box::new(partial_install()),
    };
    assert_eq!(install_failure_detail(&wrapped), expected);
}

#[test]
fn a_failure_that_renders_its_own_cause_adds_no_detail_lines() {
    let e = FirmwareInstallError::ProducedNothing { packages: 4 };
    assert!(install_failure_detail(&e).is_empty());
}
