//! Staging discipline, the per-version gate, and the commit rename.

use std::collections::BTreeMap;

use super::*;
use crate::manifest::sha256_of;
use crate::scratch_dir::scratch;
use crate::store::layout::TitleId;
use crate::test_support::RecordingReporter;

fn codes(phases: &[FirmwarePhase]) -> Vec<u8> {
    phases.iter().map(|p| p.code()).collect()
}

fn digest(byte: u8) -> manifest::Sha256 {
    manifest::Sha256(sha256_of(&[byte]))
}

fn firmware_record(version: &str, pup: manifest::Sha256) -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::Firmware,
            version: version.to_string(),
            store_path: format!("firmware/{version}"),
        },
        source: SourceRecord::local(FIRMWARE_SOURCE_KIND, pup),
        title: None,
        files: BTreeMap::new(),
        rap: None,
    }
}

fn write_record(path: &Path, record: &InstallRecord) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, record.to_toml().unwrap()).unwrap();
}

fn staged_at(root: &Path, version: &str) -> (PathBuf, Staged) {
    let layout = StoreLayout::new(root);
    let artifact = Artifact::Firmware {
        version: VersionKey::new(version).unwrap(),
    };
    let entry_dir = layout.entry_dir(&artifact);
    let staging_root = layout.firmware_staging_dir();
    std::fs::create_dir_all(staging_root.join(FLASH_MOUNT)).unwrap();
    std::fs::write(
        staging_root.join(FLASH_MOUNT).join(MANIFEST_FILE),
        b"staged",
    )
    .unwrap();
    (
        staging_root,
        Staged {
            record: firmware_record(version, digest(1)),
            version: version.to_string(),
            record_path: layout.record_path(&artifact),
            entry_dir,
            manifest_entries: 0,
            omissions: Vec::new(),
            files: 1,
            packages: Vec::new(),
            replaced: false,
        },
    )
}

fn pup_entry(entry_id: u64, data_offset: u64, data_length: u64) -> pup::PupFileEntry {
    pup::PupFileEntry {
        entry_id,
        data_offset,
        data_length,
        _padding: [0u8; 8],
    }
}

/// The hash table is not read by the entry-extent path under test.
fn pup_with(entries: Vec<pup::PupFileEntry>) -> pup::Pup {
    pup::Pup {
        image_version: 0,
        entries,
        hashes: Vec::new(),
    }
}

fn outer_entry(name: &str) -> tar::TarEntry {
    tar::TarEntry {
        name: name.to_string(),
        data: Vec::new(),
    }
}

#[test]
fn the_update_files_payload_is_the_extent_the_entry_declares() {
    let data: Vec<u8> = (0u8..16).collect();
    let pup = pup_with(vec![
        pup_entry(0x100, 0, 4),
        pup_entry(ENTRY_ID_UPDATE_FILES, 4, 8),
    ]);
    assert_eq!(update_files_payload(&data, &pup).unwrap(), &data[4..12]);
}

#[test]
fn a_pup_with_no_update_files_entry_is_refused_by_entry_id() {
    let pup = pup_with(vec![pup_entry(0x100, 0, 4)]);
    assert!(matches!(
        update_files_payload(&[0u8; 16], &pup),
        Err(FirmwareInstallError::NoUpdateFiles)
    ));
}

#[test]
fn an_update_files_extent_past_the_file_is_not_reported_as_an_absent_entry() {
    let pup = pup_with(vec![pup_entry(ENTRY_ID_UPDATE_FILES, 8, 32)]);
    let err = update_files_payload(&[0u8; 16], &pup).expect_err("the extent leaves the buffer");
    let FirmwareInstallError::UpdateFilesOutOfBounds {
        offset,
        length,
        file_len,
    } = &err
    else {
        panic!("expected UpdateFilesOutOfBounds, got {err}");
    };
    assert_eq!((*offset, *length, *file_len), (8, 32, 16));
}

#[test]
fn the_dev_flash3_package_is_not_a_payload_package() {
    let outer = vec![
        outer_entry("dev_flash_000.tar"),
        outer_entry("dev_flash3_000.tar"),
        outer_entry("dev_flash_001.tar"),
    ];
    let picked = dev_flash_packages(&outer).expect("two payload packages");
    assert_eq!(
        picked.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
        ["dev_flash_000.tar", "dev_flash_001.tar"]
    );
}

#[test]
fn an_outer_tar_with_no_payload_package_is_refused_before_anything_is_staged() {
    let outer = vec![
        outer_entry("dev_flash3_000.tar"),
        outer_entry("spkg_hdr.tar"),
    ];
    assert!(matches!(
        dev_flash_packages(&outer),
        Err(FirmwareInstallError::NoDevFlashPackages)
    ));
}

#[test]
fn the_staging_directory_is_a_hidden_sibling_of_the_entries_it_becomes() {
    let layout = StoreLayout::new("vfs");
    let staging = layout.firmware_staging_dir();
    assert_eq!(staging.parent(), Some(layout.firmware_root().as_path()));
    let entry = layout.entry_dir(&Artifact::Firmware {
        version: VersionKey::new("4.91").unwrap(),
    });
    // The commit rename has to stay inside one directory, so the two
    // share a parent.
    assert_eq!(staging.parent(), entry.parent());
}

#[test]
fn preparing_staging_sweeps_an_interrupted_installs_residue() {
    let dir = scratch();
    let staging = StoreLayout::new(&*dir).firmware_staging_dir();
    std::fs::create_dir_all(staging.join("dev_flash/sys")).unwrap();
    std::fs::write(staging.join("dev_flash/sys/leftover.sprx"), b"old").unwrap();

    let reporter = RecordingReporter::default();
    prepare_staging(&staging, &reporter).expect("prepare");
    assert!(staging.is_dir(), "the staging root is recreated");
    assert_eq!(
        std::fs::read_dir(&staging).unwrap().count(),
        0,
        "no residue survives into the commit rename"
    );
    assert_eq!(reporter.phases(), codes(&[FirmwarePhase::ClearingStaging]));
}

#[test]
fn preparing_a_fresh_staging_directory_announces_no_sweep() {
    let dir = scratch();
    let staging = StoreLayout::new(&*dir).firmware_staging_dir();
    let reporter = RecordingReporter::default();
    prepare_staging(&staging, &reporter).expect("prepare");
    assert!(staging.is_dir());
    assert!(reporter.phases().is_empty());
}

#[test]
fn a_failed_stage_discards_the_whole_staging_tree() {
    let dir = scratch();
    let staging = StoreLayout::new(&*dir).firmware_staging_dir();
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("half-written.bin"), b"x").unwrap();

    let err = run_or_clean(&staging, || {
        Err::<(), _>(FirmwareInstallError::ProducedNothing { packages: 3 })
    })
    .expect_err("the fault propagates");
    assert!(matches!(
        err,
        FirmwareInstallError::ProducedNothing { packages: 3 }
    ));
    assert!(!staging.exists(), "nothing is left behind");
}

#[test]
fn an_empty_or_absent_entry_directory_reads_as_free() {
    let dir = scratch();
    assert!(!dir_non_empty(&dir.join("absent")).unwrap());
    let empty = dir.join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(!dir_non_empty(&empty).unwrap());
    std::fs::write(empty.join("x"), b"x").unwrap();
    assert!(dir_non_empty(&empty).unwrap());
}

#[test]
fn a_record_that_is_there_and_unreadable_is_not_absence() {
    let dir = scratch();
    let path = dir.join("bad.install.toml");
    std::fs::write(&path, "format_version = 99\n").unwrap();
    assert!(matches!(
        read_record(&path),
        Err(FirmwareInstallError::RecordParse { .. })
    ));
    assert!(read_record(&dir.join("absent.install.toml"))
        .unwrap()
        .is_none());
}

#[test]
fn an_unrecorded_version_installs_and_an_unrecorded_tree_is_refused() {
    let dir = scratch();
    let record = dir.join("firmware.install.toml");
    let entry = dir.join("4.91");
    assert!(!check_entry(&record, &entry, "4.91", digest(1), false).unwrap());

    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(entry.join("residue.bin"), b"x").unwrap();
    assert!(matches!(
        check_entry(&record, &entry, "4.91", digest(1), false),
        Err(FirmwareInstallError::TargetExists { .. })
    ));
    // Residue is not an installed version, so --force does not report a
    // replacement.
    assert!(!check_entry(&record, &entry, "4.91", digest(1), true).unwrap());
}

#[test]
fn reinstalling_the_same_pup_names_the_shared_source_hash() {
    let dir = scratch();
    let record = dir.join("4.91.install.toml");
    write_record(&record, &firmware_record("4.91", digest(1)));

    let err = check_entry(&record, &dir.join("4.91"), "4.91", digest(1), false)
        .expect_err("an installed version is refused");
    let FirmwareInstallError::VersionInstalled { pup_sha256, .. } = &err else {
        panic!("expected VersionInstalled, got {err}");
    };
    assert_eq!(*pup_sha256, digest(1));
    assert!(err.to_string().contains("this same PUP"), "{err}");
    assert!(check_entry(&record, &dir.join("4.91"), "4.91", digest(1), true).unwrap());
}

#[test]
fn a_second_pup_under_one_version_string_is_refused_naming_both_hashes() {
    let dir = scratch();
    let record = dir.join("4.91.install.toml");
    write_record(&record, &firmware_record("4.91", digest(1)));

    let err = check_entry(&record, &dir.join("4.91"), "4.91", digest(2), false)
        .expect_err("a different PUP under the same version is refused");
    let FirmwareInstallError::VersionInstalledFromAnotherPup {
        installed,
        incoming,
        ..
    } = &err
    else {
        panic!("expected VersionInstalledFromAnotherPup, got {err}");
    };
    assert_eq!(*installed, digest(1));
    assert_eq!(*incoming, digest(2));
    let rendered = err.to_string();
    assert!(rendered.contains(&digest(1).to_hex()), "{rendered}");
    assert!(rendered.contains(&digest(2).to_hex()), "{rendered}");
}

#[test]
fn a_record_describing_something_else_is_refused_before_force_can_replace_it() {
    let dir = scratch();
    let record = dir.join("4.91.install.toml");
    let mut other = firmware_record("3.55", digest(1));
    write_record(&record, &other);
    assert!(matches!(
        check_entry(&record, &dir.join("4.91"), "4.91", digest(1), true),
        Err(FirmwareInstallError::RecordMismatch { .. })
    ));

    other.artifact.kind = ArtifactKind::TitleUpdate;
    other.artifact.version = "4.91".to_string();
    other.title = Some(crate::store::record::TitleRecord {
        title_id: TitleId::new("TEST00001").unwrap().as_str().to_string(),
        content_id: "TEST00001".to_string(),
        category: "GD".to_string(),
        title: "T".to_string(),
        distribution: "update-pkg".to_string(),
    });
    write_record(&record, &other);
    assert!(matches!(
        check_entry(&record, &dir.join("4.91"), "4.91", digest(1), true),
        Err(FirmwareInstallError::RecordMismatch { .. })
    ));
}

#[test]
fn the_commit_renames_the_tree_into_place_and_writes_the_record_after_it() {
    let dir = scratch();
    let (staging, staged) = staged_at(&dir, "4.91");
    let reporter = RecordingReporter::default();
    commit(&staging, &staged, &reporter).expect("commit");

    assert!(!staging.exists(), "the staging root is consumed");
    assert_eq!(
        std::fs::read(staged.entry_dir.join(FLASH_MOUNT).join(MANIFEST_FILE)).unwrap(),
        b"staged",
    );
    let text = std::fs::read_to_string(&staged.record_path).unwrap();
    let parsed = InstallRecord::parse(&text).expect("the record round-trips");
    assert_eq!(parsed.artifact.kind, ArtifactKind::Firmware);
    assert_eq!(parsed.artifact.version, "4.91");
    assert_eq!(parsed.artifact.store_path, "firmware/4.91");
    assert!(parsed.title.is_none(), "a firmware record names no title");
    assert!(
        parsed.files.is_empty(),
        "firmware.toml is the file manifest"
    );
    assert_eq!(reporter.phases(), codes(&[FirmwarePhase::Committing]));
}

#[test]
fn a_commit_that_cannot_rename_names_both_paths_and_the_retry() {
    let dir = scratch();
    let (staging, staged) = staged_at(&dir, "4.93");
    std::fs::remove_dir_all(&staging).unwrap();

    let err = commit(&staging, &staged, &()).expect_err("the rename cannot land");
    let FirmwareInstallError::CommitFailed {
        staging_root,
        entry_dir,
        ..
    } = &err
    else {
        panic!("expected CommitFailed, got {err}");
    };
    assert_eq!(staging_root, &staging);
    assert_eq!(entry_dir, &staged.entry_dir);
    let rendered = err.to_string();
    assert!(rendered.contains("retrying is safe"), "{rendered}");
    assert!(
        !staged.record_path.exists(),
        "a commit that never renamed writes no record"
    );
}

#[test]
fn committing_over_an_installed_version_clears_it_first() {
    let dir = scratch();
    let (staging, staged) = staged_at(&dir, "4.91");
    std::fs::create_dir_all(staged.entry_dir.join(FLASH_MOUNT)).unwrap();
    std::fs::write(
        staged.entry_dir.join(FLASH_MOUNT).join("stale.sprx"),
        b"old",
    )
    .unwrap();

    let reporter = RecordingReporter::default();
    commit(&staging, &staged, &reporter).expect("commit");
    assert!(
        !staged
            .entry_dir
            .join(FLASH_MOUNT)
            .join("stale.sprx")
            .exists(),
        "the replaced version is removed whole, not merged into"
    );
    assert_eq!(
        reporter.phases(),
        codes(&[
            FirmwarePhase::Committing,
            FirmwarePhase::Clearing,
            FirmwarePhase::Committing,
        ])
    );
}

#[test]
fn a_partial_extraction_is_refused_rather_than_committed() {
    let tally = ExtractTally {
        files: 12,
        packages: vec![PackageSummary {
            package: "dev_flash_000.tar".to_string(),
            written: 12,
            pruned: 0,
            skipped: 0,
        }],
        failed: vec![PackageFailure::InnerTar {
            package: "dev_flash_001.tar".to_string(),
            source: crate::tar::TarParseError::NotUstarHeader { offset: 0 },
        }],
        extract_errors: Vec::new(),
    };
    let err = tally
        .into_complete(2)
        .expect_err("a lost package fails the install");
    let FirmwareInstallError::PartialInstall {
        files,
        packages,
        packages_failed,
        ..
    } = &err
    else {
        panic!("expected PartialInstall, got {err}");
    };
    assert_eq!((*files, *packages, packages_failed.len()), (12, 2, 1));
    assert!(
        err.to_string().contains("dev_flash_001.tar")
            || packages_failed[0].to_string().contains("dev_flash_001.tar"),
        "the lost package is named"
    );
}

#[test]
fn a_lost_entry_write_fails_the_install_even_when_every_package_opened() {
    let tally = ExtractTally {
        files: 5,
        packages: vec![PackageSummary {
            package: "dev_flash_000.tar".to_string(),
            written: 5,
            pruned: 0,
            skipped: 0,
        }],
        failed: Vec::new(),
        extract_errors: vec![crate::tar::ExtractError::PathTraversal {
            guest_path: "../escape".to_string(),
            host_path: PathBuf::from("escape"),
        }],
    };
    let err = tally
        .into_complete(1)
        .expect_err("a lost entry fails the install");
    let FirmwareInstallError::PartialInstall {
        packages_failed,
        extract_errors,
        ..
    } = &err
    else {
        panic!("expected PartialInstall, got {err}");
    };
    assert!(packages_failed.is_empty());
    assert_eq!(extract_errors.len(), 1);
    assert!(err.to_string().contains("1 entry write(s) failed"), "{err}");
}

#[test]
fn an_extraction_that_wrote_nothing_is_refused() {
    let err = ExtractTally::default()
        .into_complete(4)
        .expect_err("zero files is not a successful install");
    assert!(matches!(
        err,
        FirmwareInstallError::ProducedNothing { packages: 4 }
    ));
}

#[test]
fn a_complete_extraction_passes_through_unchanged() {
    let tally = ExtractTally {
        files: 3,
        packages: vec![PackageSummary {
            package: "dev_flash_000.tar".to_string(),
            written: 3,
            pruned: 1,
            skipped: 2,
        }],
        failed: Vec::new(),
        extract_errors: Vec::new(),
    };
    let ok = tally.into_complete(1).expect("nothing was lost");
    assert_eq!(ok.files, 3);
    assert_eq!(ok.packages.len(), 1);
}
