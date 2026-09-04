use super::*;

use std::collections::BTreeMap;

use crate::scratch_dir::{scratch, ScratchDir};
use crate::store::record::{ArtifactRecord, RapRecord, TitleRecord};
use crate::store::{ArtifactKind, INSTALL_RECORD_FORMAT_VERSION};

/// Placeholder identity: these cases build every tree by hand and name
/// no installed corpus.
const SYNTHETIC_TITLE_ID: &str = "TEST00000";

fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
    }
    std::fs::write(path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// A tree that holds `files`, and a record that lists them under the
/// hashes they were written with.
fn tree_and_record(files: &[(&str, &[u8])]) -> (ScratchDir, InstallRecord) {
    let dir = scratch();
    let mut recorded = BTreeMap::new();
    for (rel, bytes) in files {
        write(&dir.join(rel), bytes);
        recorded.insert((*rel).to_string(), sha256_of(bytes));
    }
    let record = InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "01.00".to_string(),
            store_path: format!("dev_hdd0/game/{SYNTHETIC_TITLE_ID}"),
        },
        source: crate::store::SourceRecord::local("pkg", sha256_of(b"container")),
        title: Some(TitleRecord {
            title_id: SYNTHETIC_TITLE_ID.to_string(),
            content_id: SYNTHETIC_TITLE_ID.to_string(),
            category: "HG".to_string(),
            title: "Synthetic".to_string(),
            distribution: "psn-hdd".to_string(),
        }),
        files: recorded,
        rap: None,
    };
    (dir, record)
}

#[test]
fn an_intact_tree_is_clean() {
    let (dir, record) = tree_and_record(&[("USRDIR/EBOOT.BIN", b"eboot"), ("PARAM.SFO", b"sfo")]);
    let report = verify_record_tree(&dir, None, &record).expect("read the tree");
    assert!(report.is_clean(), "divergences: {:?}", report.divergences);
    assert_eq!(report.matched, 2);
    assert_eq!(report.checked(), 2);
}

#[test]
fn a_rewritten_file_reports_both_hashes() {
    let (dir, record) = tree_and_record(&[("USRDIR/EBOOT.BIN", b"eboot")]);
    write(&dir.join("USRDIR/EBOOT.BIN"), b"tampered");
    let report = verify_record_tree(&dir, None, &record).expect("read the tree");
    let [only] = report.divergences.as_slice() else {
        panic!("expected one divergence, got {:?}", report.divergences);
    };
    assert_eq!(
        only.kind,
        DivergenceKind::Modified {
            expected: sha256_of(b"eboot"),
            found: sha256_of(b"tampered"),
        }
    );
    assert_eq!(report.matched, 0);
}

#[test]
fn a_deleted_file_is_missing_rather_than_the_empty_hash() {
    let (dir, record) = tree_and_record(&[("PARAM.SFO", b"sfo"), ("USRDIR/EBOOT.BIN", b"eboot")]);
    std::fs::remove_file(dir.join("PARAM.SFO")).expect("remove the recorded file");
    let report = verify_record_tree(&dir, None, &record).expect("read the tree");
    let [only] = report.divergences.as_slice() else {
        panic!("expected one divergence, got {:?}", report.divergences);
    };
    assert_eq!(only.kind, DivergenceKind::Missing);
    assert_eq!(report.matched, 1, "the intact file still counts as matched");
}

#[test]
fn a_zero_byte_file_is_matched_by_its_own_hash() {
    let (dir, record) = tree_and_record(&[("USRDIR/placeholder.edat", b"")]);
    let report = verify_record_tree(&dir, None, &record).expect("read the tree");
    assert!(report.is_clean(), "divergences: {:?}", report.divergences);

    std::fs::remove_file(dir.join("USRDIR/placeholder.edat")).expect("remove the placeholder");
    let report = verify_record_tree(&dir, None, &record).expect("read the tree");
    assert_eq!(
        report.divergences.first().map(|d| &d.kind),
        Some(&DivergenceKind::Missing)
    );
}

/// Removal of the directory that holds the recorded files is the same
/// act as removal of each file in it.
#[test]
fn an_absent_tree_names_every_recorded_file_as_missing() {
    let (dir, record) = tree_and_record(&[("PARAM.SFO", b"sfo"), ("USRDIR/EBOOT.BIN", b"eboot")]);
    let gone = dir.join("no-such-entry");
    let report = verify_record_tree(&gone, None, &record).expect("probe the absent tree");
    assert_eq!(report.checked(), 2, "both recorded files are accounted for");
    assert!(!report.is_clean(), "a tree that is gone whole is not clean");
    assert!(
        report
            .divergences
            .iter()
            .all(|d| d.kind == DivergenceKind::Missing),
        "{:?}",
        report.divergences
    );
}

/// The RAP lives outside the entry directory, so an absent tree says
/// nothing about it either way.
#[test]
fn an_absent_tree_still_holds_the_rap_against_the_record() {
    let (dir, mut record) = tree_and_record(&[("PARAM.SFO", b"sfo")]);
    let rap = dir.join("exdata").join("license.rap");
    write(&rap, b"a different rap!");
    record.rap = Some(RapRecord {
        filename: "license.rap".to_string(),
        sha256: sha256_of(b"sixteen-bytes!!!"),
    });

    let gone = dir.join("no-such-entry");
    let report = verify_record_tree(&gone, Some(&rap), &record).expect("read the rap");
    assert_eq!(report.checked(), 2, "the recorded file and the RAP");
    assert!(
        report
            .divergences
            .iter()
            .any(|d| d.path == rap && matches!(d.kind, DivergenceKind::Modified { .. })),
        "{:?}",
        report.divergences
    );
}

#[test]
fn a_recorded_path_that_cannot_be_read_is_neither_a_match_nor_a_divergence() {
    let (dir, record) = tree_and_record(&[("USRDIR/EBOOT.BIN", b"eboot")]);
    let at = dir.join("USRDIR/EBOOT.BIN");
    std::fs::remove_file(&at).expect("clear the recorded file");
    // A directory where the record names a file: every host refuses to
    // read it, and none of them reports the refusal as absence.
    std::fs::create_dir(&at).expect("put a directory in its place");

    let err = verify_record_tree(&dir, None, &record).expect_err("a directory is not a file read");
    assert_eq!(err.path, at);
    assert_ne!(err.source.kind(), std::io::ErrorKind::NotFound);
}

#[test]
fn an_unreadable_rap_is_reported_rather_than_read_as_absent() {
    let (dir, mut record) = tree_and_record(&[("PARAM.SFO", b"sfo")]);
    let rap = dir.join("exdata").join("license.rap");
    std::fs::create_dir_all(&rap).expect("put a directory where the RAP belongs");
    record.rap = Some(RapRecord {
        filename: "license.rap".to_string(),
        sha256: sha256_of(b"sixteen-bytes!!!"),
    });

    let err = verify_record_tree(&dir, Some(&rap), &record).expect_err("the RAP read fails");
    assert_eq!(err.path, rap);
}

#[test]
fn a_recorded_rap_is_checked_when_it_is_on_disk() {
    let (dir, mut record) = tree_and_record(&[("PARAM.SFO", b"sfo")]);
    let rap = dir.join("exdata").join("license.rap");
    write(&rap, b"sixteen-bytes!!!");
    record.rap = Some(RapRecord {
        filename: "license.rap".to_string(),
        sha256: sha256_of(b"sixteen-bytes!!!"),
    });

    let report = verify_record_tree(&dir, Some(&rap), &record).expect("read the tree");
    assert_eq!(report.matched, 2, "the RAP counts as a checked artefact");

    write(&rap, b"a different rap!");
    let report = verify_record_tree(&dir, Some(&rap), &record).expect("read the tree");
    assert_eq!(report.divergences.len(), 1);
    assert_eq!(report.divergences[0].path, rap);
}

#[test]
fn a_rap_the_live_directory_no_longer_holds_is_not_a_divergence() {
    let (dir, mut record) = tree_and_record(&[("PARAM.SFO", b"sfo")]);
    record.rap = Some(RapRecord {
        filename: "license.rap".to_string(),
        sha256: sha256_of(b"sixteen-bytes!!!"),
    });
    let absent = dir.join("exdata").join("license.rap");
    let report = verify_record_tree(&dir, Some(&absent), &record).expect("read the tree");
    assert!(report.is_clean(), "divergences: {:?}", report.divergences);
    assert_eq!(report.checked(), 1, "only the recorded file was checked");
}

#[test]
fn a_divergence_renders_the_path_and_both_hashes() {
    let rendered = Divergence {
        path: PathBuf::from("USRDIR/EBOOT.BIN"),
        kind: DivergenceKind::Modified {
            expected: sha256_of(b"eboot"),
            found: sha256_of(b"tampered"),
        },
    }
    .to_string();
    assert!(rendered.contains("USRDIR/EBOOT.BIN"), "{rendered}");
    assert!(
        rendered.contains(&sha256_of(b"eboot").to_hex()),
        "{rendered}"
    );
    assert!(
        rendered.contains(&sha256_of(b"tampered").to_hex()),
        "{rendered}"
    );
}
