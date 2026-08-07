//! Game-install rejection paths that fire before the EBOOT NPD parse
//! (synthetic fixtures cannot produce a decryptable EBOOT, so the
//! happy path is covered by the presence-gated real-dump parity test
//! in `tests/parity_pkg.rs`).

use super::*;
use crate::test_support::{
    build_iso, build_npdrm_eboot_header, build_param_sfo, build_pkg, pkg_file, scratch, IsoNode,
    PkgItem,
};
use std::path::PathBuf;

const KLIC: [u8; 16] = [
    0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01,
];

fn sfo_item(entries: &[(&str, &str)]) -> PkgItem {
    pkg_file("PARAM.SFO", 3, &build_param_sfo(entries))
}

#[test]
fn rejects_missing_param_sfo() {
    let pkg = build_pkg(&KLIC, "NPUA80001", &[pkg_file("README.TXT", 3, b"hi")]);
    let out = scratch();
    let err = install_pkg(&pkg, None, &out.join("vfs"), &out.join("installs"), false).unwrap_err();
    assert!(matches!(err, GameInstallError::NoParamSfo));
}

#[test]
fn rejects_non_hdd_category() {
    let pkg = build_pkg(
        &KLIC,
        "NPUA80001",
        &[sfo_item(&[("TITLE_ID", "NPUA80001"), ("CATEGORY", "GD")])],
    );
    let out = scratch();
    let err = install_pkg(&pkg, None, &out.join("vfs"), &out.join("installs"), false).unwrap_err();
    assert!(matches!(err, GameInstallError::NotHddGame { category } if category == "GD"));
}

#[test]
fn rejects_title_id_mismatch() {
    // Header title-id NPUA80001, but PARAM.SFO claims NPUA80068.
    let pkg = build_pkg(
        &KLIC,
        "NPUA80001",
        &[sfo_item(&[("TITLE_ID", "NPUA80068"), ("CATEGORY", "HG")])],
    );
    let out = scratch();
    let err = install_pkg(&pkg, None, &out.join("vfs"), &out.join("installs"), false).unwrap_err();
    assert!(matches!(err, GameInstallError::TitleIdMismatch { .. }));
}

#[test]
fn rejects_missing_title_id() {
    let pkg = build_pkg(
        &KLIC,
        "NPUA80001",
        &[sfo_item(&[("CATEGORY", "HG"), ("TITLE", "flOw")])],
    );
    let out = scratch();
    let err = install_pkg(&pkg, None, &out.join("vfs"), &out.join("installs"), false).unwrap_err();
    assert!(matches!(err, GameInstallError::MissingTitleId));
}

#[test]
fn rejects_missing_eboot() {
    let pkg = build_pkg(
        &KLIC,
        "NPUA80001",
        &[sfo_item(&[("TITLE_ID", "NPUA80001"), ("CATEGORY", "HG")])],
    );
    let out = scratch();
    let err = install_pkg(&pkg, None, &out.join("vfs"), &out.join("installs"), false).unwrap_err();
    assert!(matches!(err, GameInstallError::NoEboot));
    // A rejection before staging must leave no game tree behind.
    assert!(!out.join("vfs/dev_hdd0/game/NPUA80001").exists());
}

// --- Disc (ISO) install rejection paths -------------------------------

#[test]
fn iso_rejects_missing_param_sfo() {
    let image = build_iso(vec![IsoNode::File("PS3_DISC.SFB", b"sfb".to_vec())]);
    let out = scratch();
    let err = install_iso(
        &image,
        &image,
        &out.join("vfs"),
        &out.join("installs"),
        false,
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::NoDiscParamSfo));
}

#[test]
fn iso_rejects_non_disc_category() {
    let image = build_iso(vec![IsoNode::Dir(
        "PS3_GAME",
        vec![IsoNode::File(
            "PARAM.SFO",
            build_param_sfo(&[("TITLE_ID", "BCES00664"), ("CATEGORY", "HG")]),
        )],
    )]);
    let out = scratch();
    let err = install_iso(
        &image,
        &image,
        &out.join("vfs"),
        &out.join("installs"),
        false,
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::NotDiscGame { category } if category == "HG"));
}

#[test]
fn iso_rejects_missing_eboot() {
    let image = build_iso(vec![IsoNode::Dir(
        "PS3_GAME",
        vec![IsoNode::File(
            "PARAM.SFO",
            build_param_sfo(&[("TITLE_ID", "BCES00664"), ("CATEGORY", "DG")]),
        )],
    )]);
    let out = scratch();
    let err = install_iso(
        &image,
        &image,
        &out.join("vfs"),
        &out.join("installs"),
        false,
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::NoDiscEboot));
    assert!(!out.join("vfs/dev_bdvd/BCES00664").exists());
}

// --- Input sanitizers -------------------------------------------------

#[test]
fn safe_join_accepts_normal_nested_path() {
    let base = Path::new("base");
    assert_eq!(
        safe_join(base, "USRDIR/EBOOT.BIN").unwrap(),
        base.join("USRDIR").join("EBOOT.BIN")
    );
    // A leading `./` component is dropped, not an error.
    assert_eq!(
        safe_join(base, "./PARAM.SFO").unwrap(),
        base.join("PARAM.SFO")
    );
}

#[test]
fn safe_join_rejects_parent_traversal() {
    assert!(matches!(
        safe_join(Path::new("base"), "../escape.bin").unwrap_err(),
        GameInstallError::UnsafeEntryPath { path } if path == "../escape.bin"
    ));
}

#[test]
fn safe_join_rejects_absolute_entry() {
    assert!(matches!(
        safe_join(Path::new("base"), "/abs/payload").unwrap_err(),
        GameInstallError::UnsafeEntryPath { .. }
    ));
}

#[test]
fn validate_content_id_accepts_real_ids() {
    validate_content_id("NPUA80001").unwrap();
    validate_content_id("UP9000-NPUA80001_00-FLOWPS3PROMOTION").unwrap();
}

#[test]
fn validate_content_id_rejects_slash_and_empty() {
    assert!(matches!(
        validate_content_id("UP9000/NPUA80001").unwrap_err(),
        GameInstallError::UnsafeContentId { .. }
    ));
    assert!(matches!(
        validate_content_id("").unwrap_err(),
        GameInstallError::UnsafeContentId { .. }
    ));
}

/// A base v2 record with the given `rap`, for round-trip tests.
fn sample_record(rap: Option<RapRecord>) -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        source: SourceRecord {
            kind: "pkg".to_string(),
            sha256: sha256_of(b"source bytes"),
        },
        title: TitleRecord {
            title_id: "NPUA80001".to_string(),
            content_id: "UP9000-NPUA80001_00-FLOWPS3PROMOTION".to_string(),
            category: "HG".to_string(),
            title: "flOw".to_string(),
            app_version: "01.00".to_string(),
            distribution: "psn-hdd".to_string(),
        },
        files: std::collections::BTreeMap::from([(
            "USRDIR/EBOOT.BIN".to_string(),
            sha256_of(b"eboot bytes"),
        )]),
        rap,
    }
}

#[test]
fn install_record_round_trips_with_rap_present() {
    let record = sample_record(Some(RapRecord {
        filename: "UP9000-NPUA80001_00-FLOWPS3PROMOTION.rap".to_string(),
        sha256: sha256_of(b"rap bytes"),
    }));
    let text = toml::to_string(&record).expect("serialise");
    // The compact form is a single [files] table, not a [[files]] array.
    assert!(text.contains("[files]"));
    assert!(!text.contains("[[files]]"));
    assert!(
        text.contains("[rap]"),
        "RAP-present record emits a [rap] table"
    );
    let back: InstallRecord = toml::from_str(&text).expect("parse");
    assert_eq!(back.title.title_id, "NPUA80001");
    assert_eq!(back.files.len(), 1);
    assert!(back.files.contains_key("USRDIR/EBOOT.BIN"));
    assert_eq!(
        back.rap.as_ref().map(|r| r.filename.as_str()),
        Some("UP9000-NPUA80001_00-FLOWPS3PROMOTION.rap")
    );
}

#[test]
fn install_record_round_trips_with_rap_absent() {
    let record = sample_record(None);
    let text = toml::to_string(&record).expect("serialise");
    // A RAP-absent record omits the [rap] table entirely.
    assert!(
        !text.contains("[rap]"),
        "RAP-absent record has no [rap] table"
    );
    let back: InstallRecord = toml::from_str(&text).expect("parse");
    assert!(back.rap.is_none());
}

// --- RAP contract + pre-commit residue (NPDRM EBOOT fixtures) ---------

const NPD_TITLE_ID: &str = "NPUA80001";
const NPD_CONTENT_ID: &str = "UP9000-NPUA80001_00-TEST";

/// A synthetic HG PKG whose EBOOT carries an NPD header of `license`.
/// The EBOOT is not a real SELF, so the decrypt-proof fails -- which is
/// what the residue assertions rely on (a pre-commit fault).
fn npdrm_pkg(license: u32) -> Vec<u8> {
    let sfo = build_param_sfo(&[("TITLE_ID", NPD_TITLE_ID), ("CATEGORY", "HG")]);
    let eboot = build_npdrm_eboot_header(license, NPD_CONTENT_ID);
    build_pkg(
        &KLIC,
        NPD_CONTENT_ID,
        &[
            pkg_file("PARAM.SFO", 3, &sfo),
            pkg_file("USRDIR/EBOOT.BIN", 1, &eboot),
        ],
    )
}

fn exdata_rap(vfs: &Path) -> PathBuf {
    vfs.join(format!(
        "dev_hdd0/home/00000001/exdata/{NPD_CONTENT_ID}.rap"
    ))
}

#[test]
fn pre_commit_fault_leaves_no_exdata_residue() {
    // Network license + a 16-byte RAP: the RAP stages under the root,
    // the proof fails on the synthetic EBOOT, and the batch is
    // discarded -- nothing reaches exdata. (Reverting the
    // stage-RAP-under-root change, which wrote the RAP to live exdata
    // before the proof, turns this red.)
    let pkg = npdrm_pkg(1);
    let out = scratch();
    let vfs = out.join("vfs");
    let err = install_pkg(&pkg, Some(&[0u8; 16]), &vfs, &out.join("installs"), false).unwrap_err();
    assert!(
        matches!(err, GameInstallError::DecryptProof(_)),
        "synthetic EBOOT must fail the proof, got {err:?}"
    );
    assert!(!exdata_rap(&vfs).exists(), "no RAP residue in exdata");
    assert!(
        !vfs.join("dev_hdd0/game/.staging-NPUA80001").exists(),
        "staging dir discarded"
    );
}

#[test]
fn rejects_rap_required_for_network_license() {
    let pkg = npdrm_pkg(1); // network
    let out = scratch();
    let err = install_pkg(&pkg, None, &out.join("vfs"), &out.join("installs"), false).unwrap_err();
    assert!(
        matches!(err, GameInstallError::RapRequired { content_id } if content_id == NPD_CONTENT_ID)
    );
}

#[test]
fn rejects_wrong_size_rap() {
    let pkg = npdrm_pkg(1);
    let out = scratch();
    let err = install_pkg(
        &pkg,
        Some(&[0u8; 15]),
        &out.join("vfs"),
        &out.join("installs"),
        false,
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::RapWrongSize { len: 15 }));
}

#[test]
fn rap_consumed_only_for_network_and_local() {
    use crate::npdrm::NpdLicense;
    assert!(!rap_consumed(None)); // APP-keyed, no NPD header
    assert!(!rap_consumed(Some(NpdLicense::Free))); // the finding-6 case
    assert!(rap_consumed(Some(NpdLicense::Network)));
    assert!(rap_consumed(Some(NpdLicense::Local)));
}

#[test]
fn free_license_with_supplied_rap_plans_no_rap() {
    // The finding-6 witness: a free title plans no RAP even when one is
    // supplied. Collapsing the license gate back to a RAP-presence test
    // (the bug) flips this red -- the FS residue tests cannot, since a
    // failed proof discards either way.
    let plan = plan_staged_rap(false, Some(&[0u8; 16]), "X", Path::new("s"), Path::new("e"));
    assert!(plan.is_none(), "free license plans no staged RAP");
}

#[test]
fn network_license_plans_a_rap() {
    let plan = plan_staged_rap(true, Some(&[0u8; 16]), "X", Path::new("s"), Path::new("e"));
    let sr = plan.expect("network license plans a staged RAP");
    assert_eq!(sr.staged_path, Path::new("s").join("rap").join("X.rap"));
    assert_eq!(sr.final_path, Path::new("e").join("X.rap"));
}

#[test]
fn rejects_existing_target_without_force_and_force_bypasses() {
    let pkg = npdrm_pkg(1);
    let out = scratch();
    let vfs = out.join("vfs");
    let final_dir = vfs.join("dev_hdd0/game/NPUA80001");
    std::fs::create_dir_all(&final_dir).unwrap();
    std::fs::write(final_dir.join("old"), b"x").unwrap();

    // force=false: the non-empty target is rejected before staging.
    let err = install_pkg(&pkg, Some(&[0u8; 16]), &vfs, &out.join("installs"), false).unwrap_err();
    assert!(matches!(err, GameInstallError::TargetExists { .. }));

    // force=true: the guard is bypassed; the synthetic EBOOT then fails
    // the proof (not TargetExists), proving force got past the gate.
    let err = install_pkg(&pkg, Some(&[0u8; 16]), &vfs, &out.join("installs"), true).unwrap_err();
    assert!(
        matches!(err, GameInstallError::DecryptProof(_)),
        "force must bypass TargetExists, got {err:?}"
    );
}

#[test]
fn prepare_staging_clears_stale_content() {
    let out = scratch();
    let staging = out.join(".staging-X");
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("foreign.bin"), b"old").unwrap();
    prepare_staging(&staging).unwrap();
    assert!(staging.exists());
    assert!(
        !staging.join("foreign.bin").exists(),
        "stale staging content is cleared"
    );
}

#[test]
fn prepare_staging_surfaces_non_notfound_removal_error() {
    // A regular file where prepare_staging expects to remove a dir:
    // remove_dir_all returns a non-NotFound error, which must surface
    // rather than be swallowed (the swallow was the pre-fix bug). NotFound
    // (the dir simply absent) stays the silent, fine case.
    let out = scratch();
    let staging = out.join(".staging-X");
    std::fs::write(&staging, b"not a dir").unwrap();
    let err = prepare_staging(&staging).unwrap_err();
    assert!(
        matches!(err, GameInstallError::Io { op: "remove", .. }),
        "non-NotFound removal error must surface, got {err:?}"
    );
}

#[test]
fn normalized_rel_strips_curdir_and_keeps_real_paths() {
    assert_eq!(normalized_rel("USRDIR/EBOOT.BIN"), "USRDIR/EBOOT.BIN");
    assert_eq!(normalized_rel("./PARAM.SFO"), "PARAM.SFO");
    assert_eq!(normalized_rel("A/./B"), "A/B");
}

#[test]
fn build_record_is_deterministic_and_sorted() {
    let staged = vec![
        StagedFile {
            path: "USRDIR/EBOOT.BIN".to_string(),
            is_dir: false,
            data: b"eboot".to_vec(),
        },
        StagedFile {
            path: "PARAM.SFO".to_string(),
            is_dir: false,
            data: b"sfo".to_vec(),
        },
        StagedFile {
            path: "USRDIR".to_string(),
            is_dir: true,
            data: Vec::new(),
        },
    ];
    let mk = || {
        build_record(
            "pkg",
            b"src-bytes",
            &staged,
            TitleRecord {
                title_id: "NPUA80001".to_string(),
                content_id: "UP9000-NPUA80001_00-TEST".to_string(),
                category: "HG".to_string(),
                title: "T".to_string(),
                app_version: "01.00".to_string(),
                distribution: "psn-hdd".to_string(),
            },
            Some(RapRecord {
                filename: "UP9000-NPUA80001_00-TEST.rap".to_string(),
                sha256: sha256_of(b"rap"),
            }),
        )
    };
    let a = toml::to_string(&mk()).expect("serialise");
    let b = toml::to_string(&mk()).expect("serialise");
    assert_eq!(a, b, "record TOML is a pure function of its inputs");
    // Files are sorted by path regardless of staged order.
    assert!(
        a.find("\"PARAM.SFO\"").unwrap() < a.find("\"USRDIR/EBOOT.BIN\"").unwrap(),
        "[files] keys are sorted"
    );
}
