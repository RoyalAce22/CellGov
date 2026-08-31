//! The update installer: what it accepts, what it refuses, and what a
//! committed update entry holds. Synthetic update PKGs reach the happy
//! path here because an update install runs no decrypt-proof.

#![cfg_attr(not(feature = "decrypt"), allow(unused_imports, dead_code))]

use super::*;
use crate::game_install::staging::sha256_of;
use crate::scratch_dir::scratch;
use crate::store::record::INSTALL_RECORD_FORMAT_VERSION;
use crate::test_support::build_param_sfo;
#[cfg(feature = "decrypt")]
use crate::test_support::{build_pkg, pkg_file};
use std::collections::BTreeMap;

const TITLE_ID: &str = "BCES00664";
const CONTENT_ID: &str = "EP9000-BCES00664_00-WIPEOUTHD0000000";
const VERSION: &str = "02.51";

const KLIC: [u8; 16] = [
    0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01,
];

#[cfg(feature = "decrypt")]
fn keys() -> KeyVault {
    crate::test_support::synthetic_vault()
}

#[cfg(feature = "decrypt")]
fn update_pkg(category: &str, app_ver: &str, eboot: &[u8]) -> Vec<u8> {
    let sfo = build_param_sfo(&[
        ("TITLE_ID", TITLE_ID),
        ("CATEGORY", category),
        ("TITLE", "WipEout HD"),
        ("APP_VER", app_ver),
    ]);
    build_pkg(
        &keys(),
        &KLIC,
        CONTENT_ID,
        &[
            pkg_file("PARAM.SFO", 3, &sfo),
            pkg_file("USRDIR", 4, &[]),
            pkg_file("USRDIR/EBOOT.BIN", 1, eboot),
        ],
    )
}

#[cfg(feature = "decrypt")]
fn install(vfs: &Path, pkg: &[u8], force: bool) -> Result<UpdateInstallOutcome, GameInstallError> {
    install_update_pkg(
        pkg,
        &keys(),
        vfs,
        InstallOptions {
            force,
            ..Default::default()
        },
    )
}

fn write_base_record(vfs: &Path, title_id: &str) -> PathBuf {
    let layout = StoreLayout::new(vfs);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new(title_id).expect("synthetic title id"),
    };
    let record = build_record(
        "iso",
        b"base-source",
        ArtifactRecord {
            kind: ArtifactKind::TitleBase,
            version: "02.00".to_string(),
            store_path: format!("titles/{title_id}/base"),
        },
        BTreeMap::new(),
        TitleRecord {
            title_id: title_id.to_string(),
            content_id: title_id.to_string(),
            category: "DG".to_string(),
            title: "WipEout HD".to_string(),
            distribution: "disc-iso".to_string(),
        },
        None,
    );
    let path = layout.record_path(&artifact);
    std::fs::create_dir_all(path.parent().expect("a record is never a root")).unwrap();
    std::fs::write(&path, record.to_toml().expect("serialise")).unwrap();
    path
}

#[cfg(feature = "decrypt")]
#[test]
fn a_gd_update_lands_under_the_version_key_with_its_tree_in_game() {
    let out = scratch();
    let vfs = out.join("vfs");
    let outcome =
        install(&vfs, &update_pkg("GD", VERSION, b"patched-eboot"), false).expect("install");

    assert_eq!(outcome.title_id, TITLE_ID);
    assert_eq!(outcome.content_id, CONTENT_ID);
    assert_eq!(outcome.version, VERSION);
    assert!(outcome.orphan, "no base installed");
    assert!(!outcome.replaced);
    assert_eq!(
        outcome.update_dir,
        vfs.join("titles")
            .join(TITLE_ID)
            .join("updates")
            .join(VERSION)
    );
    assert_eq!(
        std::fs::read(outcome.update_dir.join("game/USRDIR/EBOOT.BIN")).unwrap(),
        b"patched-eboot"
    );
    assert!(
        !vfs.join("titles")
            .join(TITLE_ID)
            .join("updates")
            .join(".staging-02.51")
            .exists(),
        "the staging root is consumed by the commit rename"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn the_record_keys_the_entry_and_its_files_under_the_version() {
    let out = scratch();
    let vfs = out.join("vfs");
    let outcome = install(&vfs, &update_pkg("GD", VERSION, b"eboot"), false).expect("install");

    assert_eq!(
        outcome.record_path,
        vfs.join(".cellgov/installs/titles")
            .join(TITLE_ID)
            .join("update-02.51.install.toml")
    );
    let record = InstallRecord::parse(&std::fs::read_to_string(&outcome.record_path).unwrap())
        .expect("the committed record parses");
    assert_eq!(record.format_version, INSTALL_RECORD_FORMAT_VERSION);
    assert_eq!(record.artifact.kind, ArtifactKind::TitleUpdate);
    assert_eq!(record.artifact.version, VERSION);
    assert_eq!(record.artifact.store_path, "titles/BCES00664/updates/02.51");
    let title = record.title.expect("a title-update record carries [title]");
    assert_eq!(title.title_id, TITLE_ID);
    assert_eq!(title.category, "GD");
    assert_eq!(title.distribution, UPDATE_DISTRIBUTION);
    assert!(record.rap.is_none(), "an update installs no RAP");
    // Keys stay relative to the entry the record names, so `game/` is
    // part of every key rather than implied by the reader.
    assert_eq!(
        record.files.keys().collect::<Vec<_>>(),
        vec!["game/PARAM.SFO", "game/USRDIR/EBOOT.BIN"]
    );
    assert_eq!(outcome.file_count, 2);
}

#[cfg(feature = "decrypt")]
#[test]
fn an_hg_update_is_accepted_too() {
    let out = scratch();
    let vfs = out.join("vfs");
    let outcome = install(&vfs, &update_pkg("HG", "01.01", b"eboot"), false).expect("install");
    assert_eq!(outcome.version, "01.01");
    assert!(outcome.update_dir.join("game/PARAM.SFO").exists());
}

#[cfg(feature = "decrypt")]
#[test]
fn a_category_that_is_neither_gd_nor_hg_is_not_an_update() {
    let out = scratch();
    let err = install(&out.join("vfs"), &update_pkg("DG", VERSION, b"e"), false).unwrap_err();
    assert!(
        matches!(&err, GameInstallError::NotUpdatePackage { category } if category == "DG"),
        "{err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_pkg_whose_header_does_not_carry_the_title_id_is_refused() {
    let sfo = build_param_sfo(&[
        ("TITLE_ID", "NPEB00001"),
        ("CATEGORY", "GD"),
        ("APP_VER", VERSION),
    ]);
    let pkg = build_pkg(
        &keys(),
        &KLIC,
        CONTENT_ID,
        &[pkg_file("PARAM.SFO", 3, &sfo)],
    );
    let out = scratch();
    let vfs = out.join("vfs");
    let err = install(&vfs, &pkg, false).unwrap_err();
    assert!(
        matches!(err, GameInstallError::TitleIdMismatch { .. }),
        "{err:?}"
    );
    assert!(
        !vfs.join("titles").exists(),
        "refused before anything was staged"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn an_update_with_no_version_string_has_no_directory_to_install_into() {
    let sfo = build_param_sfo(&[("TITLE_ID", TITLE_ID), ("CATEGORY", "GD")]);
    let pkg = build_pkg(
        &keys(),
        &KLIC,
        CONTENT_ID,
        &[pkg_file("PARAM.SFO", 3, &sfo)],
    );
    let err = install(&scratch().join("vfs"), &pkg, false).unwrap_err();
    assert!(
        matches!(err, GameInstallError::MissingAppVersion),
        "{err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_version_string_that_is_no_directory_name_is_refused_as_a_store_key() {
    let err = install(
        &scratch().join("vfs"),
        &update_pkg("GD", "../evil", b"e"),
        false,
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::StoreKey(_)), "{err:?}");
}

#[cfg(feature = "decrypt")]
#[test]
fn an_installed_version_is_refused_by_name_and_left_untouched() {
    let out = scratch();
    let vfs = out.join("vfs");
    let first = update_pkg("GD", VERSION, b"first-eboot");
    let outcome = install(&vfs, &first, false).expect("first install");

    let err = install(&vfs, &update_pkg("GD", VERSION, b"second-eboot"), false).unwrap_err();
    let GameInstallError::UpdateVersionInstalled {
        version,
        existing_source,
    } = &err
    else {
        panic!("expected UpdateVersionInstalled, got {err:?}");
    };
    assert_eq!(version, VERSION);
    assert_eq!(
        *existing_source,
        sha256_of(&first),
        "names the installed source"
    );
    assert!(
        err.to_string().contains(&existing_source.to_hex()),
        "the refusal renders the hash: {err}"
    );
    assert_eq!(
        std::fs::read(outcome.update_dir.join("game/USRDIR/EBOOT.BIN")).unwrap(),
        b"first-eboot",
        "the installed entry is immutable"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn force_replaces_an_installed_version_whole() {
    let out = scratch();
    let vfs = out.join("vfs");
    install(&vfs, &update_pkg("GD", VERSION, b"first-eboot"), false).expect("first install");
    // A file only the first install carries: --force replaces the
    // entry rather than merging into it.
    let stale = vfs
        .join("titles")
        .join(TITLE_ID)
        .join("updates")
        .join(VERSION)
        .join("game/OLD.DAT");
    std::fs::write(&stale, b"stale").unwrap();

    let second = update_pkg("GD", VERSION, b"second-eboot");
    let outcome = install(&vfs, &second, true).expect("forced install");
    assert!(outcome.replaced);
    assert_eq!(
        std::fs::read(outcome.update_dir.join("game/USRDIR/EBOOT.BIN")).unwrap(),
        b"second-eboot"
    );
    assert!(!stale.exists(), "the replaced entry is gone whole");

    let record = InstallRecord::parse(&std::fs::read_to_string(&outcome.record_path).unwrap())
        .expect("record");
    assert_eq!(record.source.sha256, sha256_of(&second));
}

#[cfg(feature = "decrypt")]
#[test]
fn an_entry_directory_with_no_record_is_refused_as_an_occupied_target() {
    let out = scratch();
    let vfs = out.join("vfs");
    let entry = vfs
        .join("titles")
        .join(TITLE_ID)
        .join("updates")
        .join(VERSION);
    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(entry.join("residue"), b"x").unwrap();

    let err = install(&vfs, &update_pkg("GD", VERSION, b"e"), false).unwrap_err();
    assert!(
        matches!(err, GameInstallError::TargetExists { .. }),
        "{err:?}"
    );
    assert!(
        entry.join("residue").exists(),
        "the refusal removed nothing"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn staging_residue_from_an_interrupted_install_is_swept_by_name() {
    let out = scratch();
    let vfs = out.join("vfs");
    let staging = vfs
        .join("titles")
        .join(TITLE_ID)
        .join("updates")
        .join(".staging-02.51");
    std::fs::create_dir_all(staging.join("game/USRDIR")).unwrap();
    std::fs::write(staging.join("game/USRDIR/HALF.DAT"), b"interrupted").unwrap();

    let outcome = install(&vfs, &update_pkg("GD", VERSION, b"eboot"), false).expect("install");
    assert!(
        !outcome.update_dir.join("game/USRDIR/HALF.DAT").exists(),
        "residue from the interrupted install must not ride into the commit"
    );
    assert!(!staging.exists(), "the staging root is consumed");
}

#[cfg(feature = "decrypt")]
#[test]
fn a_base_record_for_this_title_clears_the_orphan_flag() {
    let out = scratch();
    let vfs = out.join("vfs");
    write_base_record(&vfs, TITLE_ID);
    let outcome = install(&vfs, &update_pkg("GD", VERSION, b"e"), false).expect("install");
    assert!(!outcome.orphan, "the base is installed");
}

#[cfg(feature = "decrypt")]
#[test]
fn a_base_record_naming_another_title_refuses_the_store_entry() {
    let out = scratch();
    let vfs = out.join("vfs");
    // A record filed under this title's entry that describes a
    // different title: the store entry is not the one the update
    // belongs to.
    let path = write_base_record(&vfs, TITLE_ID);
    let text = std::fs::read_to_string(&path)
        .unwrap()
        .replace(TITLE_ID, "NPEB00001");
    std::fs::write(&path, text).unwrap();

    let err = install(&vfs, &update_pkg("GD", VERSION, b"e"), false).unwrap_err();
    let GameInstallError::BaseRecordMismatch {
        found, expected, ..
    } = &err
    else {
        panic!("expected BaseRecordMismatch, got {err:?}");
    };
    assert_eq!(found, "NPEB00001");
    assert_eq!(expected, TITLE_ID);
    assert!(!vfs.join("titles").join(TITLE_ID).join("updates").exists());
}

#[test]
fn a_base_record_that_will_not_parse_is_named_rather_than_ignored() {
    let out = scratch();
    let path = out.join("base.install.toml");
    std::fs::write(&path, "format_version = 1\n").unwrap();
    let err = check_base_entry(&path, TITLE_ID).unwrap_err();
    assert!(
        matches!(err, GameInstallError::RecordParse { .. }),
        "{err:?}"
    );
}

#[test]
fn an_absent_base_record_reads_as_an_orphan_update() {
    let out = scratch();
    assert!(check_base_entry(&out.join("nothing.toml"), TITLE_ID).unwrap());
}

#[test]
fn a_base_record_declaring_an_update_entry_is_not_a_base() {
    let out = scratch();
    let path = out.join("base.install.toml");
    let record = build_record(
        "pkg",
        b"update-source",
        ArtifactRecord {
            kind: ArtifactKind::TitleUpdate,
            version: VERSION.to_string(),
            store_path: format!("titles/{TITLE_ID}/updates/{VERSION}"),
        },
        BTreeMap::new(),
        TitleRecord {
            title_id: TITLE_ID.to_string(),
            content_id: TITLE_ID.to_string(),
            category: "GD".to_string(),
            title: "WipEout HD".to_string(),
            distribution: UPDATE_DISTRIBUTION.to_string(),
        },
        None,
    );
    std::fs::write(&path, record.to_toml().expect("serialise")).unwrap();

    let err = check_base_entry(&path, TITLE_ID).unwrap_err();
    let GameInstallError::BaseRecordMismatch { kind, found, .. } = &err else {
        panic!("expected BaseRecordMismatch, got {err:?}");
    };
    assert_eq!(*kind, ArtifactKind::TitleUpdate);
    assert_eq!(found, TITLE_ID, "the title matches; the kind does not");
}

#[cfg(feature = "decrypt")]
#[test]
fn an_update_record_that_will_not_parse_refuses_the_install_under_force_too() {
    let out = scratch();
    let vfs = out.join("vfs");
    let record_path = vfs
        .join(".cellgov/installs/titles")
        .join(TITLE_ID)
        .join("update-02.51.install.toml");
    std::fs::create_dir_all(record_path.parent().expect("a record is never a root")).unwrap();
    std::fs::write(&record_path, "format_version = 1\n").unwrap();

    // --force replaces an entry the store can read; it is not a licence
    // to write over a record whose contents are unknown.
    for force in [false, true] {
        let err = install(&vfs, &update_pkg("GD", VERSION, b"e"), force).unwrap_err();
        assert!(
            matches!(&err, GameInstallError::RecordParse { path, .. } if path == &record_path),
            "force={force}: {err:?}"
        );
    }
    assert!(
        !vfs.join("titles").join(TITLE_ID).join("updates").exists(),
        "refused before anything was staged"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn every_record_file_key_resolves_under_the_entry_the_record_names() {
    let out = scratch();
    let vfs = out.join("vfs");
    let outcome =
        install(&vfs, &update_pkg("GD", VERSION, b"patched-eboot"), false).expect("install");
    let record = InstallRecord::parse(&std::fs::read_to_string(&outcome.record_path).unwrap())
        .expect("record");

    // The uninstall verifier joins each `[files]` key onto the directory
    // `store_path` resolves to, so the two must agree about `game/`.
    assert_eq!(
        StoreLayout::new(&vfs).resolve_store_path(&record.artifact.store_path),
        outcome.update_dir
    );
    assert!(!record.files.is_empty());
    for (rel, expected) in &record.files {
        let path = outcome.update_dir.join(rel);
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("{} names no committed file: {e}", path.display()));
        assert_eq!(sha256_of(&bytes), *expected, "{rel}");
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn an_empty_entry_directory_is_not_an_occupied_target() {
    let out = scratch();
    let vfs = out.join("vfs");
    let entry = vfs
        .join("titles")
        .join(TITLE_ID)
        .join("updates")
        .join(VERSION);
    std::fs::create_dir_all(&entry).unwrap();

    let outcome = install(&vfs, &update_pkg("GD", VERSION, b"eboot"), false).expect("install");
    assert!(!outcome.replaced);
    assert!(outcome.update_dir.join("game/PARAM.SFO").exists());
}

#[cfg(feature = "decrypt")]
#[test]
fn force_over_an_unrecorded_entry_replaces_the_tree_and_reports_no_replaced_version() {
    let out = scratch();
    let vfs = out.join("vfs");
    let entry = vfs
        .join("titles")
        .join(TITLE_ID)
        .join("updates")
        .join(VERSION);
    std::fs::create_dir_all(&entry).unwrap();
    std::fs::write(entry.join("residue"), b"x").unwrap();

    let outcome =
        install(&vfs, &update_pkg("GD", VERSION, b"eboot"), true).expect("forced install");
    assert!(
        !entry.join("residue").exists(),
        "the entry is replaced whole"
    );
    assert!(
        !outcome.replaced,
        "an installed version is one with a record, so unrecorded residue is not a replaced version"
    );
}

/// The `game/` prefix must not widen what the installer accepts.
/// `pkg::extract` stops a rooted name and any `..` component before
/// the installer sees it, so the reachable case is a bare `.`: it
/// normalizes to nothing on its own and is refused, but prefixed it
/// reads as the path `game` and would stage a file where the tree
/// directory belongs.
#[cfg(feature = "decrypt")]
#[test]
fn a_curdir_entry_is_refused_rather_than_staged_over_the_tree_directory() {
    let sfo = build_param_sfo(&[
        ("TITLE_ID", TITLE_ID),
        ("CATEGORY", "GD"),
        ("APP_VER", VERSION),
    ]);
    for entry in [".", "./"] {
        let pkg = build_pkg(
            &keys(),
            &KLIC,
            CONTENT_ID,
            &[
                pkg_file("PARAM.SFO", 3, &sfo),
                pkg_file(entry, 1, b"payload"),
            ],
        );
        let out = scratch();
        let vfs = out.join("vfs");
        let err = install(&vfs, &pkg, false).unwrap_err();
        assert!(
            matches!(&err, GameInstallError::UnsafeEntryPath { path } if path == entry),
            "{entry:?} must be refused by name, got {err:?}"
        );
        assert!(
            !vfs.join("titles")
                .join(TITLE_ID)
                .join("updates")
                .join(VERSION)
                .exists(),
            "{entry:?} left a committed entry behind"
        );
    }
}

#[cfg(feature = "decrypt")]
#[test]
fn a_refused_update_stops_the_phase_trail_where_it_faulted() {
    use crate::progress::Phase;
    use crate::test_support::{codes, RecordingReporter};

    let reporter = RecordingReporter::default();
    let out = scratch();
    let err = install_update_pkg(
        &update_pkg("DG", VERSION, b"eboot"),
        &keys(),
        &out.join("vfs"),
        InstallOptions {
            force: false,
            progress: &reporter,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, GameInstallError::NotUpdatePackage { .. }),
        "{err:?}"
    );
    assert_eq!(reporter.phases(), codes(&[Phase::Reading]));
    assert!(!reporter.finished(), "a refused install reported finished");
}

#[cfg(feature = "decrypt")]
#[test]
fn a_completed_update_walks_reading_staging_hashing_committing() {
    use crate::progress::Phase;
    use crate::test_support::{codes, RecordingReporter};

    let reporter = RecordingReporter::default();
    let out = scratch();
    install_update_pkg(
        &update_pkg("GD", VERSION, b"eboot"),
        &keys(),
        &out.join("vfs"),
        InstallOptions {
            force: false,
            progress: &reporter,
        },
    )
    .expect("install");
    assert_eq!(
        reporter.phases(),
        codes(&[
            Phase::Reading,
            Phase::Staging,
            Phase::Hashing,
            Phase::Committing,
        ]),
        "an update runs no decrypt-proof, so no Proving phase"
    );
    assert!(reporter.finished());
}
