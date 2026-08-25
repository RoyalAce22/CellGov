//! Game-install rejection paths that fire before the EBOOT NPD parse
//! (synthetic fixtures cannot produce a decryptable EBOOT, so the
//! happy path is covered by the presence-gated real-dump parity test
//! in `tests/parity_pkg.rs`).

use super::*;
use crate::scratch_dir::scratch;
use crate::test_support::{
    build_iso, build_npdrm_eboot_header, build_param_sfo, build_pkg, pkg_file, IsoNode, PkgItem,
};
use std::path::PathBuf;

const KLIC: [u8; 16] = [
    0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01,
];

fn sfo_item(entries: &[(&str, &str)]) -> PkgItem {
    pkg_file("PARAM.SFO", 3, &build_param_sfo(entries))
}

#[test]
fn install_records_resolve_inside_the_vfs_root_they_describe() {
    for root in ["vfs", "relative/nested/vfs", "/tmp/other-vfs"] {
        let root = Path::new(root);
        let dir = installs_dir(root);
        assert!(
            dir.starts_with(root),
            "{} escaped the root it describes",
            dir.display()
        );
    }
}

#[test]
fn two_vfs_roots_do_not_share_one_record_directory() {
    assert_ne!(
        installs_dir(Path::new("vfs-a")),
        installs_dir(Path::new("vfs-b"))
    );
}

/// The install side writes under this root and `cellgov_cli
/// gen-manifest` reads from it; a second literal in either crate would
/// send the reader somewhere the writer never wrote.
#[test]
fn the_default_vfs_root_is_the_directory_the_installers_write_into() {
    assert_eq!(DEFAULT_VFS_ROOT, "vfs");
    assert_eq!(
        installs_dir(Path::new(DEFAULT_VFS_ROOT)),
        Path::new("vfs").join(".cellgov").join("installs")
    );
}

/// Placeholder identity: the version gate runs before a caller reads
/// any of it, so these name no real title and no installed corpus.
const SYNTHETIC_TITLE_ID: &str = "TEST00000";
const SYNTHETIC_CONTENT_ID: &str = "TT0000-TEST00000_00-SYNTHETICRECORD0";

fn record_toml(format_version: u32) -> String {
    let record = InstallRecord {
        format_version,
        source: SourceRecord {
            kind: "pkg".to_string(),
            sha256: sha256_of(b"src"),
        },
        title: TitleRecord {
            title_id: SYNTHETIC_TITLE_ID.to_string(),
            content_id: SYNTHETIC_CONTENT_ID.to_string(),
            category: "HG".to_string(),
            title: "synthetic record".to_string(),
            app_version: "01.00".to_string(),
            distribution: "psn-hdd".to_string(),
        },
        files: BTreeMap::new(),
        rap: None,
    };
    toml::to_string(&record).unwrap()
}

#[test]
fn a_record_declaring_another_schema_version_is_refused_by_name() {
    for found in [0, 1, INSTALL_RECORD_FORMAT_VERSION + 1] {
        let err = InstallRecord::parse(&record_toml(found)).unwrap_err();
        assert!(
            matches!(
                err,
                InstallRecordParseError::UnsupportedFormatVersion { found: f, supported }
                    if f == found && supported == INSTALL_RECORD_FORMAT_VERSION
            ),
            "format_version {found} produced {err:?}"
        );
    }
}

#[test]
fn a_record_at_the_current_schema_version_parses() {
    let record = InstallRecord::parse(&record_toml(INSTALL_RECORD_FORMAT_VERSION)).unwrap();
    assert_eq!(record.format_version, INSTALL_RECORD_FORMAT_VERSION);
    assert_eq!(record.title.title_id, SYNTHETIC_TITLE_ID);
    assert_eq!(record.title.content_id, SYNTHETIC_CONTENT_ID);
}

#[test]
fn a_record_that_is_not_toml_is_refused_separately_from_a_version_mismatch() {
    let err = InstallRecord::parse("this is not toml {{{").unwrap_err();
    assert!(matches!(err, InstallRecordParseError::Toml(_)), "{err:?}");
}

#[test]
fn rejects_missing_param_sfo() {
    let pkg = build_pkg(&KLIC, "NPUA80001", &[pkg_file("README.TXT", 3, b"hi")]);
    let out = scratch();
    let err = install_pkg(
        &pkg,
        None,
        &out.join("vfs"),
        &out.join("installs"),
        InstallOptions::default(),
    )
    .unwrap_err();
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
    let err = install_pkg(
        &pkg,
        None,
        &out.join("vfs"),
        &out.join("installs"),
        InstallOptions::default(),
    )
    .unwrap_err();
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
    let err = install_pkg(
        &pkg,
        None,
        &out.join("vfs"),
        &out.join("installs"),
        InstallOptions::default(),
    )
    .unwrap_err();
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
    let err = install_pkg(
        &pkg,
        None,
        &out.join("vfs"),
        &out.join("installs"),
        InstallOptions::default(),
    )
    .unwrap_err();
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
    let err = install_pkg(
        &pkg,
        None,
        &out.join("vfs"),
        &out.join("installs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::NoEboot));
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
        InstallOptions::default(),
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
        InstallOptions::default(),
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
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::NoDiscEboot));
    assert!(!out.join("vfs/dev_bdvd/BCES00664").exists());
}

#[test]
fn iso_pre_commit_fault_leaves_no_staging_residue() {
    // On the disc path the staging root *is* the tree (no `tree/`
    // nesting), so a failed decrypt-proof has to discard the root
    // itself. The EBOOT is not a SELF, so the proof faults.
    let image = build_iso(vec![IsoNode::Dir(
        "PS3_GAME",
        vec![
            IsoNode::File(
                "PARAM.SFO",
                build_param_sfo(&[("TITLE_ID", "BCES00664"), ("CATEGORY", "DG")]),
            ),
            IsoNode::Dir(
                "USRDIR",
                vec![IsoNode::File("EBOOT.BIN", b"not a SELF".to_vec())],
            ),
        ],
    )]);
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    let err = install_iso(&image, &image, &vfs, &installs, InstallOptions::default()).unwrap_err();
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
        !installs.join("BCES00664.install.toml").exists(),
        "no record for a failed install"
    );
}

// --- Input sanitizers -------------------------------------------------

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
    // discarded -- nothing reaches exdata.
    let pkg = npdrm_pkg(1);
    let out = scratch();
    let vfs = out.join("vfs");
    let err = install_pkg(
        &pkg,
        Some(&[0u8; 16]),
        &vfs,
        &out.join("installs"),
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

#[test]
fn rejects_rap_required_for_network_license() {
    let pkg = npdrm_pkg(1); // network
    let out = scratch();
    let err = install_pkg(
        &pkg,
        None,
        &out.join("vfs"),
        &out.join("installs"),
        InstallOptions::default(),
    )
    .unwrap_err();
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
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::RapWrongSize { len: 15 }));
}

#[test]
fn rap_consumed_only_for_network_and_local() {
    use crate::npdrm::NpdLicense;
    assert!(!rap_consumed(None)); // APP-keyed, no NPD header
    assert!(!rap_consumed(Some(NpdLicense::Free))); // NP_KLIC_FREE fallback
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
    // A free title resolves through NP_KLIC_FREE, so a supplied RAP is
    // dropped at the plan and never staged, committed, or recorded.
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
        &vfs,
        &out.join("installs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, GameInstallError::TargetExists { .. }));

    // force=true: the synthetic EBOOT reaches the decrypt proof and
    // fails there.
    let err = install_pkg(
        &pkg,
        Some(&[0u8; 16]),
        &vfs,
        &out.join("installs"),
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
    // remove_dir_all returns a non-NotFound error, which must surface.
    // NotFound (the dir simply absent) stays the silent, fine case.
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
            digests,
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
    let a = toml::to_string(&mk(&out.join("a"))).expect("serialise");
    let b = toml::to_string(&mk(&out.join("b"))).expect("serialise");
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
        assert!(!content_id_is_safe(bad), "{bad:?} must be refused");
    }
    for ok in ["NPUA80001", "UP9000-NPUA80001_00-TEST", "BCES00664"] {
        assert!(content_id_is_safe(ok), "{ok:?} must be accepted");
    }
}

/// A reporter that counts everything, for the equivalence tests.
#[derive(Default)]
struct CountingReporter {
    totals_bytes: std::sync::atomic::AtomicU64,
    totals_files: std::sync::atomic::AtomicUsize,
    bytes: std::sync::atomic::AtomicU64,
    /// `bytes_advanced` calls: one per written piece.
    pieces: std::sync::atomic::AtomicUsize,
    started: std::sync::atomic::AtomicUsize,
    finished: std::sync::atomic::AtomicUsize,
}

impl crate::progress::InstallProgress for CountingReporter {
    fn phase(&self, _phase: crate::progress::Phase) {}
    fn totals(&self, files: usize, bytes: u64) {
        self.totals_files
            .store(files, std::sync::atomic::Ordering::Relaxed);
        self.totals_bytes
            .store(bytes, std::sync::atomic::Ordering::Relaxed);
    }
    fn file_started(&self, _path: &str) {
        self.started
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn bytes_advanced(&self, delta: u64) {
        self.bytes
            .fetch_add(delta, std::sync::atomic::Ordering::Relaxed);
        self.pieces
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    fn file_finished(&self) {
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
    phases: std::sync::Mutex<Vec<crate::progress::Phase>>,
    finished: std::sync::atomic::AtomicBool,
}

impl RecordingReporter {
    fn phases(&self) -> Vec<crate::progress::Phase> {
        self.phases.lock().unwrap().clone()
    }
}

impl crate::progress::InstallProgress for RecordingReporter {
    fn phase(&self, phase: crate::progress::Phase) {
        self.phases.lock().unwrap().push(phase);
    }
    fn totals(&self, _files: usize, _bytes: u64) {}
    fn file_started(&self, _path: &str) {}
    fn bytes_advanced(&self, _delta: u64) {}
    fn file_finished(&self) {}
    fn finished(&self) {
        self.finished
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// `finished` means the install completed; a renderer draws its 100%
/// frame on it. A pre-commit fault must therefore leave it unset, and
/// the phase trail must stop at the phase that faulted.
#[test]
fn a_pre_commit_fault_never_reports_finished() {
    use crate::progress::Phase;
    let pkg = npdrm_pkg(1);
    let out = scratch();
    let reporter = RecordingReporter::default();
    let err = install_pkg(
        &pkg,
        Some(&[0u8; 16]),
        &out.join("vfs"),
        &out.join("installs"),
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
        vec![Phase::Reading, Phase::Staging, Phase::Proving]
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
    let record = sample_record(None);
    let installs = out.join("installs");
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
            &installs,
            SYNTHETIC_TITLE_ID,
            &record,
            &reporter,
        )
        .expect("commit");
        reporter.phases()
    };

    let fresh = out.join("fresh");
    assert_eq!(
        run(&fresh, &out.join(".staging-fresh")),
        vec![Phase::Committing]
    );
    assert!(fresh.join("new").exists());

    let existing = out.join("existing");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("old"), b"o").unwrap();
    assert_eq!(
        run(&existing, &out.join(".staging-existing")),
        vec![Phase::Committing, Phase::Clearing, Phase::Committing]
    );
    assert!(
        !existing.join("old").exists(),
        "old target content survived"
    );
    assert!(existing.join("new").exists());
}
