//! Game-install rejection paths that fire before the EBOOT NPD parse
//! (synthetic fixtures cannot produce a decryptable EBOOT, so the
//! happy path is covered by the presence-gated real-dump parity test
//! in `tests/parity_pkg.rs`).

#![cfg_attr(not(feature = "decrypt"), allow(unused_imports, dead_code))]

use super::*;
use crate::scratch_dir::scratch;
use crate::store::{ArtifactKind, ArtifactRecord};
use crate::test_support::{build_iso, build_npdrm_eboot_header, build_param_sfo, IsoNode};
#[cfg(feature = "decrypt")]
use crate::test_support::{build_pkg, pkg_file, PkgItem};
use std::path::PathBuf;

const KLIC: [u8; 16] = [
    0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01,
];

#[cfg(feature = "decrypt")]
fn sfo_item(entries: &[(&str, &str)]) -> PkgItem {
    pkg_file("PARAM.SFO", 3, &build_param_sfo(entries))
}

#[cfg(feature = "decrypt")]
fn keys() -> KeyVault {
    crate::test_support::synthetic_vault()
}

#[cfg(feature = "decrypt")]
#[test]
fn rejects_missing_param_sfo() {
    let pkg = build_pkg(
        &keys(),
        &KLIC,
        "NPUA80001",
        &[pkg_file("README.TXT", 3, b"hi")],
    );
    let out = scratch();
    let err = install_pkg(
        &pkg,
        None,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::NoParamSfo));
}

#[cfg(feature = "decrypt")]
#[test]
fn rejects_non_hdd_category() {
    let pkg = build_pkg(
        &keys(),
        &KLIC,
        "NPUA80001",
        &[sfo_item(&[("TITLE_ID", "NPUA80001"), ("CATEGORY", "GD")])],
    );
    let out = scratch();
    let err = install_pkg(
        &pkg,
        None,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::NotHddGame { category } if category == "GD"));
}

#[cfg(feature = "decrypt")]
#[test]
fn rejects_title_id_mismatch() {
    // Header title-id NPUA80001, but PARAM.SFO claims NPUA80068.
    let pkg = build_pkg(
        &keys(),
        &KLIC,
        "NPUA80001",
        &[sfo_item(&[("TITLE_ID", "NPUA80068"), ("CATEGORY", "HG")])],
    );
    let out = scratch();
    let err = install_pkg(
        &pkg,
        None,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::TitleIdMismatch { .. }));
}

#[cfg(feature = "decrypt")]
#[test]
fn rejects_missing_title_id() {
    let pkg = build_pkg(
        &keys(),
        &KLIC,
        "NPUA80001",
        &[sfo_item(&[("CATEGORY", "HG"), ("TITLE", "flOw")])],
    );
    let out = scratch();
    let err = install_pkg(
        &pkg,
        None,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::MissingTitleId));
}

#[cfg(feature = "decrypt")]
#[test]
fn rejects_missing_eboot() {
    let pkg = build_pkg(
        &keys(),
        &KLIC,
        "NPUA80001",
        &[sfo_item(&[("TITLE_ID", "NPUA80001"), ("CATEGORY", "HG")])],
    );
    let out = scratch();
    let err = install_pkg(
        &pkg,
        None,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::NoEboot));
    assert!(!out.join("vfs/dev_hdd0/game/NPUA80001").exists());
}

#[test]
fn parse_identity_requires_title_id_and_prefers_app_ver_over_version() {
    let err = parse_identity(&build_param_sfo(&[("CATEGORY", "HG")])).unwrap_err();
    assert!(matches!(err, GameInstallError::MissingTitleId), "{err:?}");

    let (title_id, category, title, app_version) = parse_identity(&build_param_sfo(&[
        ("TITLE_ID", "NPUA80001"),
        ("CATEGORY", "HG"),
        ("TITLE", "flOw"),
        ("VERSION", "01.02"),
    ]))
    .unwrap();
    assert_eq!(title_id, "NPUA80001");
    assert_eq!(category, "HG");
    assert_eq!(title, "flOw");
    assert_eq!(
        app_version, "01.02",
        "VERSION stands in when APP_VER is absent"
    );

    let (_, category, title, app_version) = parse_identity(&build_param_sfo(&[
        ("TITLE_ID", "NPUA80001"),
        ("APP_VER", "01.05"),
        ("VERSION", "01.02"),
    ]))
    .unwrap();
    assert_eq!(app_version, "01.05", "APP_VER wins over VERSION");
    assert_eq!(
        category, "",
        "an absent CATEGORY reads as empty; the category gate refuses it downstream"
    );
    assert_eq!(title, "");
}

// --- Disc (ISO) install rejection paths -------------------------------

#[cfg(feature = "decrypt")]
#[test]
fn iso_rejects_missing_param_sfo() {
    let image = build_iso(vec![IsoNode::File("PS3_DISC.SFB", b"sfb".to_vec())]);
    let out = scratch();
    let err =
        install_iso(&image, &keys(), &out.join("vfs"), InstallOptions::default()).unwrap_err();
    assert!(matches!(err, GameInstallError::NoDiscParamSfo));
}

#[cfg(feature = "decrypt")]
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
    let err =
        install_iso(&image, &keys(), &out.join("vfs"), InstallOptions::default()).unwrap_err();
    assert!(matches!(err, GameInstallError::NotDiscGame { category } if category == "HG"));
}

#[cfg(feature = "decrypt")]
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
    let err =
        install_iso(&image, &keys(), &out.join("vfs"), InstallOptions::default()).unwrap_err();
    assert!(matches!(err, GameInstallError::NoDiscEboot));
    assert!(!out.join("vfs/dev_bdvd/BCES00664").exists());
}

#[cfg(feature = "decrypt")]
#[test]
fn iso_pre_commit_fault_leaves_no_staging_residue() {
    // On the disc path the staging root *is* the tree (no `tree/`
    // nesting), so a failed decrypt-proof has to discard the root
    // itself. The EBOOT opens with the SCE magic (so it passes the
    // encrypted-image check) but is no SELF, so the proof faults.
    let mut eboot = cellgov_ps3_abi::sce::SCE_MAGIC.to_vec();
    eboot.extend_from_slice(b" not a SELF");
    let image = build_iso(vec![IsoNode::Dir(
        "PS3_GAME",
        vec![
            IsoNode::File(
                "PARAM.SFO",
                build_param_sfo(&[("TITLE_ID", "BCES00664"), ("CATEGORY", "DG")]),
            ),
            IsoNode::Dir("USRDIR", vec![IsoNode::File("EBOOT.BIN", eboot)]),
        ],
    )]);
    let out = scratch();
    let vfs = out.join("vfs");
    let err = install_iso(&image, &keys(), &vfs, InstallOptions::default()).unwrap_err();
    assert!(
        matches!(err, GameInstallError::DecryptProof(_)),
        "synthetic disc EBOOT must fail the proof, got {err:?}"
    );
    assert!(
        !vfs.join("dev_bdvd/.staging-BCES00664").exists(),
        "staging root discarded"
    );
    assert!(
        !vfs.join("dev_bdvd/BCES00664").exists(),
        "nothing committed"
    );
    assert!(
        !vfs.join(".cellgov").exists(),
        "no record for a failed install"
    );
}

// --- Input sanitizers -------------------------------------------------

/// Whatever an entry stages as, its `[files]` key has to be one the
/// record gate accepts -- otherwise an install writes a record it
/// cannot read back. `Path::components` splits `\` and a drive prefix
/// on Win32 and not on a POSIX host, so which names reach the gate is
/// host-dependent.
#[test]
fn every_entry_staging_accepts_yields_a_record_key_the_gate_accepts() {
    let base = Path::new("base");
    for entry in [
        "PARAM.SFO",
        "USRDIR/EBOOT.BIN",
        "USRDIR//EBOOT.BIN",
        "USRDIR/./EBOOT.BIN",
        "USRDIR/a b/c.dat",
        r"USRDIR\EBOOT.BIN",
        "USRDIR/a:stream",
        "C:/absolute",
        "../escape",
        "/rooted",
        "",
    ] {
        if safe_join(base, entry).is_ok() {
            let key = normalized_rel(entry);
            assert!(
                crate::store::record::tree_rel_path_is_safe(&key),
                "{entry:?} staged as key {key:?}, which the record gate refuses"
            );
        }
    }
}

#[test]
fn safe_join_accepts_normal_nested_path() {
    let base = Path::new("base");
    assert_eq!(
        safe_join(base, "USRDIR/EBOOT.BIN").unwrap(),
        base.join("USRDIR").join("EBOOT.BIN")
    );
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

#[test]
fn validate_content_id_rejects_a_dot_prefixed_id_that_would_escape_the_mount() {
    // `..` joins to the mount root itself, which commit() removes
    // wholesale under --force; `.` collapses to the game directory.
    for id in ["..", ".", "...", ".staging-NPUA80001"] {
        assert!(
            matches!(
                validate_content_id(id).unwrap_err(),
                GameInstallError::UnsafeContentId { .. }
            ),
            "{id:?} must be refused as a path component"
        );
    }
}

// --- RAP contract + pre-commit residue (NPDRM EBOOT fixtures) ---------

const NPD_TITLE_ID: &str = "NPUA80001";
const NPD_CONTENT_ID: &str = "UP9000-NPUA80001_00-TEST";

/// A synthetic HG PKG whose EBOOT carries an NPD header of `license`.
/// The EBOOT is not a real SELF, so the decrypt-proof fails -- which is
/// what the residue assertions rely on (a pre-commit fault).
#[cfg(feature = "decrypt")]
fn npdrm_pkg(license: u32) -> Vec<u8> {
    let sfo = build_param_sfo(&[("TITLE_ID", NPD_TITLE_ID), ("CATEGORY", "HG")]);
    let eboot = build_npdrm_eboot_header(license, NPD_CONTENT_ID);
    build_pkg(
        &keys(),
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

#[cfg(feature = "decrypt")]
#[test]
fn pre_commit_fault_leaves_no_exdata_residue() {
    // Network license + a 16-byte RAP: the RAP stages under the root,
    // the proof fails on the synthetic EBOOT, and the batch is
    // discarded -- nothing reaches exdata.
    let pkg = npdrm_pkg(1);
    let out = scratch();
    let vfs = out.join("vfs");
    let err = install_pkg(
        &pkg,
        Some(&[0u8; 16]),
        &keys(),
        &vfs,
        InstallOptions::default(),
    )
    .unwrap_err();
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

#[cfg(feature = "decrypt")]
#[test]
fn rejects_rap_required_for_network_license() {
    let pkg = npdrm_pkg(1); // network
    let out = scratch();
    let err = install_pkg(
        &pkg,
        None,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(
        matches!(err, GameInstallError::RapRequired { content_id } if content_id == NPD_CONTENT_ID)
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn rejects_wrong_size_rap() {
    let pkg = npdrm_pkg(1);
    let out = scratch();
    let err = install_pkg(
        &pkg,
        Some(&[0u8; 15]),
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::RapWrongSize { len: 15 }));
}

#[test]
fn rap_consumed_only_for_network_and_local() {
    use crate::npdrm::NpdLicense;
    assert!(!rap_consumed(None)); // APP-keyed, no NPD header
    assert!(!rap_consumed(Some(NpdLicense::Free))); // free-klicensee fallback
    assert!(rap_consumed(Some(NpdLicense::Network)));
    assert!(rap_consumed(Some(NpdLicense::Local)));
}

/// The staged-RAP plan for `license` with `rap` supplied, driven
/// through the same license gate `install_pkg` uses.
fn plan_for(license: Option<crate::npdrm::NpdLicense>, rap: Option<&[u8]>) -> Option<StagedRap> {
    plan_staged_rap(
        rap_consumed(license),
        rap,
        "X",
        Path::new("s"),
        Path::new("e"),
    )
}

#[test]
fn a_free_license_plans_no_rap_even_when_one_is_supplied() {
    // A free title resolves through the vault's free klicensee, so a
    // supplied RAP is dropped at the plan and never staged, committed,
    // or recorded.
    let plan = plan_for(Some(crate::npdrm::NpdLicense::Free), Some(&[0u8; 16]));
    assert!(plan.is_none(), "free license plans no staged RAP");
}

#[test]
fn an_app_keyed_title_plans_no_rap_even_when_one_is_supplied() {
    // No NPD header at all: same drop, via the `None` arm of the gate.
    assert!(plan_for(None, Some(&[0u8; 16])).is_none());
}

#[test]
fn a_network_license_plans_a_rap_under_the_staging_root() {
    let sr = plan_for(Some(crate::npdrm::NpdLicense::Network), Some(&[0u8; 16]))
        .expect("network license plans a staged RAP");
    assert_eq!(sr.staged_path, Path::new("s").join("rap").join("X.rap"));
    assert_eq!(sr.final_path, Path::new("e").join("X.rap"));
}

#[test]
fn a_local_license_plans_a_rap_under_the_staging_root() {
    let sr = plan_for(Some(crate::npdrm::NpdLicense::Local), Some(&[0u8; 16]))
        .expect("local license plans a staged RAP");
    assert_eq!(sr.staged_path, Path::new("s").join("rap").join("X.rap"));
    assert_eq!(sr.final_path, Path::new("e").join("X.rap"));
}

#[test]
#[should_panic(expected = "invariant: rap_needed without a RAP")]
fn a_consuming_license_with_no_rap_is_the_callers_invariant_to_have_refused() {
    // `install_pkg` returns RapRequired before it gets here, so the
    // plan treats the pair as unreachable rather than staging an empty
    // RAP. `rejects_rap_required_for_network_license` covers the
    // refusal that keeps it unreachable.
    let _ = plan_for(Some(crate::npdrm::NpdLicense::Network), None);
}

#[cfg(feature = "decrypt")]
#[test]
fn rejects_existing_target_without_force_and_force_bypasses() {
    let pkg = npdrm_pkg(1);
    let out = scratch();
    let vfs = out.join("vfs");
    let final_dir = vfs.join("dev_hdd0/game/NPUA80001");
    std::fs::create_dir_all(&final_dir).unwrap();
    std::fs::write(final_dir.join("old"), b"x").unwrap();

    // force=false: the non-empty target is rejected before staging.
    let err = install_pkg(
        &pkg,
        Some(&[0u8; 16]),
        &keys(),
        &vfs,
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::TargetExists { .. }));

    // force=true: the synthetic EBOOT reaches the decrypt proof and
    // fails there.
    let err = install_pkg(
        &pkg,
        Some(&[0u8; 16]),
        &keys(),
        &vfs,
        InstallOptions {
            force: true,
            ..Default::default()
        },
    )
    .unwrap_err();
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
    prepare_staging(&staging, &()).unwrap();
    assert!(staging.exists());
    assert!(
        !staging.join("foreign.bin").exists(),
        "stale staging content is cleared"
    );
}

#[test]
fn prepare_staging_reports_its_own_phase_only_when_there_is_residue() {
    use crate::progress::Phase;
    let out = scratch();

    let fresh = RecordingReporter::default();
    prepare_staging(&out.join(".staging-fresh"), &fresh).unwrap();
    assert_eq!(fresh.phases(), Vec::<u8>::new());

    let stale = out.join(".staging-stale");
    std::fs::create_dir_all(&stale).unwrap();
    let reporter = RecordingReporter::default();
    prepare_staging(&stale, &reporter).unwrap();
    assert_eq!(reporter.phases(), codes(&[Phase::ClearingStaging]));

    // A regular file at the path is residue too: the probe is an
    // existence check, so the removal error surfaces under this phase.
    let file = out.join(".staging-file");
    std::fs::write(&file, b"not a dir").unwrap();
    let on_file = RecordingReporter::default();
    prepare_staging(&file, &on_file).unwrap_err();
    assert_eq!(on_file.phases(), codes(&[Phase::ClearingStaging]));
}

#[test]
fn prepare_staging_surfaces_non_notfound_removal_error() {
    // A regular file where prepare_staging expects to remove a dir:
    // remove_dir_all returns a non-NotFound error, which must surface.
    // NotFound (the dir simply absent) stays the silent, fine case.
    let out = scratch();
    let staging = out.join(".staging-X");
    std::fs::write(&staging, b"not a dir").unwrap();
    let err = prepare_staging(&staging, &()).unwrap_err();
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
            data: StagedData::Bytes(b"eboot"),
        },
        StagedFile {
            path: "PARAM.SFO".to_string(),
            is_dir: false,
            data: StagedData::Bytes(b"sfo"),
        },
        StagedFile {
            path: "USRDIR".to_string(),
            is_dir: true,
            data: StagedData::Bytes(&[]),
        },
    ];
    let out = scratch();
    let mk = |dest: &Path| {
        let digests = stage_tree(&staged, dest, &()).expect("stage");
        build_record(
            "pkg",
            b"src-bytes",
            ArtifactRecord {
                kind: ArtifactKind::TitleBase,
                version: "01.00".to_string(),
                store_path: "dev_hdd0/game/NPUA80001".to_string(),
            },
            digests,
            TitleRecord {
                title_id: "NPUA80001".to_string(),
                content_id: "UP9000-NPUA80001_00-TEST".to_string(),
                category: "HG".to_string(),
                title: "T".to_string(),
                distribution: "psn-hdd".to_string(),
            },
            Some(RapRecord {
                filename: "UP9000-NPUA80001_00-TEST.rap".to_string(),
                sha256: sha256_of(b"rap"),
            }),
        )
    };
    let a = mk(&out.join("a")).to_toml().expect("serialise");
    let b = mk(&out.join("b")).to_toml().expect("serialise");
    assert_eq!(a, b, "record TOML is a pure function of its inputs");
    assert!(
        a.find("\"PARAM.SFO\"").unwrap() < a.find("\"USRDIR/EBOOT.BIN\"").unwrap(),
        "[files] keys are sorted"
    );
    // Directory entries carry no bytes and are not recorded, so the
    // record stays a hash of the installed files only.
    let record = mk(&out.join("c"));
    assert_eq!(record.files.len(), 2, "only the two file entries recorded");
    assert!(
        !record.files.contains_key("USRDIR"),
        "the staged directory entry is not a recorded file"
    );
    // Stream-hashed digests equal the whole-buffer hash of the same
    // bytes, so the record schema is unchanged by the streaming write.
    assert_eq!(record.files.get("PARAM.SFO"), Some(&sha256_of(b"sfo")));
}

/// Two container entries whose paths normalize to the same key stage
/// to one file (the second overwrites) and collapse to one record key,
/// so the outcome's `file_count` is drawn from the record rather than
/// from the staged-entry list, which would report two.
#[test]
fn entries_that_normalize_to_one_path_are_one_recorded_file() {
    let staged = vec![
        StagedFile {
            path: "USRDIR/EBOOT.BIN".to_string(),
            is_dir: false,
            data: StagedData::Bytes(b"first"),
        },
        StagedFile {
            path: "./USRDIR//EBOOT.BIN".to_string(),
            is_dir: false,
            data: StagedData::Bytes(b"second"),
        },
    ];
    let out = scratch();
    let digests = stage_tree(&staged, &out.join("tree"), &()).expect("stage");
    assert_eq!(
        staged.iter().filter(|f| !f.is_dir).count(),
        2,
        "two staged entries went in"
    );
    assert_eq!(
        digests.len(),
        1,
        "both entries address one file: {:?}",
        digests.keys().collect::<Vec<_>>()
    );
    // The last writer wins on disk, so the record holds its bytes.
    assert_eq!(
        digests.get("USRDIR/EBOOT.BIN"),
        Some(&sha256_of(b"second")),
        "the digest covers the bytes the tree ends up holding"
    );
}

/// The Slices variant streams extent slices in order; the file holds
/// their concatenation and the digest is over that concatenation.
#[test]
fn stage_tree_streams_extent_slices_in_order() {
    let a = vec![0xAAu8; 3000];
    let b = vec![0xBBu8; 500];
    let staged = vec![StagedFile {
        path: "DATA.BIN".to_string(),
        is_dir: false,
        data: StagedData::Slices(vec![&a, &b]),
    }];
    let out = scratch();
    let digests = stage_tree(&staged, &out.join("tree"), &()).expect("stage");

    let mut expected = a.clone();
    expected.extend_from_slice(&b);
    let written = std::fs::read(out.join("tree/DATA.BIN")).expect("staged file");
    assert_eq!(written, expected, "extent slices concatenate in order");
    assert_eq!(digests.get("DATA.BIN"), Some(&sha256_of(&expected)));
}

#[test]
fn a_cleanup_that_cannot_discard_the_staging_root_names_the_residue() {
    let out = scratch();
    // A regular file at the staging path: `remove_dir_all` refuses it
    // with something other than NotFound, which is exactly the shape of
    // a cleanup that leaves residue behind.
    let staging = out.join("not-a-staging-dir");
    std::fs::write(&staging, b"x").unwrap();

    let err = run_or_clean::<()>(&staging, || Err(GameInstallError::NoParamSfo))
        .expect_err("the pre-commit fault still fails the install");
    let GameInstallError::StagingResidue { path, cause, .. } = &err else {
        panic!("expected StagingResidue, got {err:?}");
    };
    assert_eq!(path, &staging);
    assert!(
        matches!(**cause, GameInstallError::NoParamSfo),
        "the original fault survives the wrap"
    );
    // Both halves reach the operator: what went wrong and what is left.
    let rendered = err.to_string();
    assert!(
        rendered.contains("PARAM.SFO"),
        "names the fault: {rendered}"
    );
    assert!(
        rendered.contains("could not be discarded"),
        "names the residue: {rendered}"
    );
}

#[test]
fn a_cleanup_that_succeeds_passes_the_original_fault_through_unwrapped() {
    let out = scratch();
    let staging = out.join("staging");
    std::fs::create_dir_all(staging.join("tree")).unwrap();

    let err = run_or_clean::<()>(&staging, || Err(GameInstallError::NoEboot))
        .expect_err("the pre-commit fault fails the install");
    assert!(matches!(err, GameInstallError::NoEboot), "got {err:?}");
    assert!(!staging.exists(), "the batch was discarded whole");
}

/// A zero-length file (an empty PKG entry, or an ISO record with data
/// length 0) is staged as an empty file and recorded under the
/// empty-input digest; it is neither skipped nor an error.
#[test]
fn a_zero_length_file_stages_as_an_empty_file_with_the_empty_digest() {
    let staged = vec![
        StagedFile {
            path: "EMPTY_PKG.BIN".to_string(),
            is_dir: false,
            data: StagedData::Bytes(&[]),
        },
        StagedFile {
            path: "EMPTY_ISO.BIN".to_string(),
            is_dir: false,
            data: StagedData::Slices(vec![&b""[..]]),
        },
        StagedFile {
            path: "NO_EXTENTS.BIN".to_string(),
            is_dir: false,
            data: StagedData::Slices(Vec::new()),
        },
    ];
    let out = scratch();
    let tree = out.join("tree");
    let digests = stage_tree(&staged, &tree, &()).expect("stage");
    assert_eq!(digests.len(), 3, "every zero-length file is recorded");
    for name in ["EMPTY_PKG.BIN", "EMPTY_ISO.BIN", "NO_EXTENTS.BIN"] {
        let written = std::fs::read(tree.join(name)).expect("staged file exists");
        assert!(written.is_empty(), "{name} holds no bytes");
        assert_eq!(
            digests.get(name),
            Some(&sha256_of(b"")),
            "{name} is recorded under the empty-input digest"
        );
    }
}

#[test]
fn a_leading_dot_content_id_is_not_a_usable_path_component() {
    for bad in ["", ".", "..", ".staging-NPUA80001", "a/b", "a\\b", "a b"] {
        assert!(!is_safe_component(bad), "{bad:?} must be refused");
    }
    for ok in ["NPUA80001", "UP9000-NPUA80001_00-TEST", "BCES00664"] {
        assert!(is_safe_component(ok), "{ok:?} must be accepted");
    }
}

/// A reporter that counts everything, for the equivalence tests.
#[derive(Default)]
struct CountingReporter {
    totals_bytes: std::sync::atomic::AtomicU64,
    totals_files: std::sync::atomic::AtomicUsize,
    bytes: std::sync::atomic::AtomicU64,
    /// `advanced` calls: one per written piece.
    pieces: std::sync::atomic::AtomicUsize,
    started: std::sync::atomic::AtomicUsize,
    finished: std::sync::atomic::AtomicUsize,
}

impl crate::progress::ProgressSink for CountingReporter {
    fn phase(&self, _code: u8) {}
    fn totals(&self, files: usize, bytes: u64) {
        self.totals_files
            .store(files, std::sync::atomic::Ordering::Relaxed);
        self.totals_bytes
            .store(bytes, std::sync::atomic::Ordering::Relaxed);
    }
    fn preset_done(&self, _amount: u64) {}
    fn item_started(&self, _path: &str) {
        self.started
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn advanced(&self, delta: u64) {
        self.bytes
            .fetch_add(delta, std::sync::atomic::Ordering::Relaxed);
        self.pieces
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn item_finished(&self) {
        self.finished
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn finished(&self) {}
}

/// Instrumentation must not change the artifact: the digests staged
/// with a live reporter equal the digests staged with the no-op one,
/// and the counted bytes equal the emitted totals -- drift here is
/// what makes a bar stop at 97%.
#[test]
fn a_live_reporter_observes_the_same_tree_the_noop_stages() {
    use std::sync::atomic::Ordering;
    // Cross the 1 MiB piece boundary so the sub-file split is real;
    // sit exactly on it and at zero so the boundaries are covered too.
    let big = vec![0xA5u8; PROGRESS_PIECE + 4096];
    let exact = vec![0x3Cu8; PROGRESS_PIECE];
    let small = vec![0x5Au8; 300];
    let staged = vec![
        StagedFile {
            path: "USRDIR/BIG.DAT".to_string(),
            is_dir: false,
            data: StagedData::Bytes(&big),
        },
        StagedFile {
            path: "USRDIR".to_string(),
            is_dir: true,
            data: StagedData::Bytes(&[]),
        },
        StagedFile {
            path: "SMALL.BIN".to_string(),
            is_dir: false,
            data: StagedData::Slices(vec![&small, &big[..100]]),
        },
        StagedFile {
            path: "EXACT.DAT".to_string(),
            is_dir: false,
            data: StagedData::Bytes(&exact),
        },
        StagedFile {
            path: "EMPTY.DAT".to_string(),
            is_dir: false,
            data: StagedData::Bytes(&[]),
        },
    ];
    let out = scratch();

    let silent = stage_tree(&staged, &out.join("silent"), &()).expect("stage silent");

    let reporter = CountingReporter::default();
    emit_totals(&reporter, &staged);
    let live = stage_tree(&staged, &out.join("live"), &reporter).expect("stage live");

    assert_eq!(silent, live, "reporter changed the staged digests");
    let expected_bytes = (2 * PROGRESS_PIECE + 4096 + 300 + 100) as u64;
    assert_eq!(
        reporter.totals_bytes.load(Ordering::Relaxed),
        expected_bytes,
        "totals must sum every non-directory entry's bytes"
    );
    assert_eq!(
        reporter.bytes.load(Ordering::Relaxed),
        expected_bytes,
        "advanced bytes must land exactly on the emitted total"
    );
    // big: 2 pieces; small: one per slice; exact: 1; empty: none.
    assert_eq!(
        reporter.pieces.load(Ordering::Relaxed),
        2 + 2 + 1,
        "a chunk is reported once per PROGRESS_PIECE sub-slice"
    );
    assert_eq!(reporter.totals_files.load(Ordering::Relaxed), 4);
    assert_eq!(reporter.started.load(Ordering::Relaxed), 4);
    assert_eq!(reporter.finished.load(Ordering::Relaxed), 4);
    assert_eq!(
        std::fs::metadata(out.join("live/EMPTY.DAT"))
            .expect("empty file staged")
            .len(),
        0
    );
}

/// Records the phase sequence and the completion flag; the sequencing
/// tests read it back after the install returns, so a plain `Mutex`
/// over the `Vec` is all the sharing the `Sync` bound needs.
#[derive(Default)]
struct RecordingReporter {
    phases: std::sync::Mutex<Vec<u8>>,
    finished: std::sync::atomic::AtomicBool,
}

impl RecordingReporter {
    fn phases(&self) -> Vec<u8> {
        self.phases.lock().unwrap().clone()
    }
}

impl crate::progress::ProgressSink for RecordingReporter {
    fn phase(&self, code: u8) {
        self.phases.lock().unwrap().push(code);
    }
    fn totals(&self, _files: usize, _bytes: u64) {}
    fn preset_done(&self, _amount: u64) {}
    fn item_started(&self, _path: &str) {}
    fn advanced(&self, _delta: u64) {}
    fn item_finished(&self) {}
    fn finished(&self) {
        self.finished
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

fn codes(phases: &[crate::progress::Phase]) -> Vec<u8> {
    phases.iter().map(|p| p.code()).collect()
}

/// `finished` means the install completed; a renderer draws its 100%
/// frame on it. A pre-commit fault must therefore leave it unset, and
/// the phase trail must stop at the phase that faulted.
#[cfg(feature = "decrypt")]
#[test]
fn a_pre_commit_fault_never_reports_finished() {
    use crate::progress::Phase;
    let pkg = npdrm_pkg(1);
    let out = scratch();
    let reporter = RecordingReporter::default();
    let err = install_pkg(
        &pkg,
        Some(&[0u8; 16]),
        &keys(),
        &out.join("vfs"),
        InstallOptions {
            force: false,
            progress: &reporter,
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, GameInstallError::DecryptProof(_)),
        "synthetic EBOOT must fail the proof, got {err:?}"
    );
    assert_eq!(
        reporter.phases(),
        codes(&[Phase::Reading, Phase::Staging, Phase::Proving])
    );
    assert!(
        !reporter.finished.load(std::sync::atomic::Ordering::Relaxed),
        "a faulted install reported finished"
    );
}

/// `Clearing` brackets the `remove_dir_all` of an existing target and
/// nothing else: a fresh target sees a single `Committing`, a forced
/// overwrite sees `Committing -> Clearing -> Committing`.
#[test]
fn commit_reports_clearing_only_when_a_target_already_exists() {
    use crate::progress::Phase;
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new("NPUA80001").expect("synthetic title id"),
    };
    let record = build_record(
        "pkg",
        b"src",
        ArtifactRecord {
            kind: artifact.kind(),
            version: "01.00".to_string(),
            store_path: "dev_hdd0/game/NPUA80001".to_string(),
        },
        BTreeMap::new(),
        TitleRecord {
            title_id: "NPUA80001".to_string(),
            content_id: "NPUA80001".to_string(),
            category: "HG".to_string(),
            title: "T".to_string(),
            distribution: "psn-hdd".to_string(),
        },
        None,
    );
    let record_path = layout.record_path(&artifact);
    let run = |final_dir: &Path, staging_root: &Path| {
        let tree = staging_root.join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        std::fs::write(tree.join("new"), b"n").unwrap();
        let reporter = RecordingReporter::default();
        commit(
            staging_root,
            &tree,
            final_dir,
            None,
            &record_path,
            &record,
            &reporter,
        )
        .expect("commit");
        reporter.phases()
    };

    let fresh = out.join("fresh");
    assert_eq!(
        run(&fresh, &out.join(".staging-fresh")),
        codes(&[Phase::Committing])
    );
    assert!(fresh.join("new").exists());

    let existing = out.join("existing");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("old"), b"o").unwrap();
    assert_eq!(
        run(&existing, &out.join(".staging-existing")),
        codes(&[Phase::Committing, Phase::Clearing, Phase::Committing])
    );
    assert!(
        !existing.join("old").exists(),
        "old target content survived"
    );
    assert!(existing.join("new").exists());
}

/// The store nests a record several directories below a `.cellgov` root
/// a fresh VFS has not got yet, so `commit` creates the chain.
#[test]
fn commit_writes_the_record_where_the_store_says_it_lives() {
    let out = scratch();
    let layout = StoreLayout::new(&*out);
    let artifact = Artifact::TitleBase {
        title_id: TitleId::new("NPUA80001").expect("synthetic title id"),
    };
    let record = build_record(
        "pkg",
        b"src",
        ArtifactRecord {
            kind: artifact.kind(),
            version: "01.00".to_string(),
            store_path: "dev_hdd0/game/NPUA80001".to_string(),
        },
        BTreeMap::new(),
        TitleRecord {
            title_id: "NPUA80001".to_string(),
            content_id: "NPUA80001".to_string(),
            category: "HG".to_string(),
            title: "T".to_string(),
            distribution: "psn-hdd".to_string(),
        },
        None,
    );
    let expected = layout.record_path(&artifact);
    assert!(
        !expected
            .parent()
            .expect("a record is never a root")
            .exists(),
        "the record directory starts absent"
    );

    let staging = out.join(".staging-NPUA80001");
    let tree = staging.join("tree");
    std::fs::create_dir_all(&tree).unwrap();
    std::fs::write(tree.join("new"), b"n").unwrap();
    let written = commit(
        &staging,
        &tree,
        &out.join("dev_hdd0").join("game").join("NPUA80001"),
        None,
        &expected,
        &record,
        &(),
    )
    .expect("commit");

    assert_eq!(written, expected);
    let text = std::fs::read_to_string(&expected).expect("the record was written");
    let back = InstallRecord::parse(&text).expect("the committed record parses");
    assert_eq!(back.artifact.store_path, "dev_hdd0/game/NPUA80001");
}
