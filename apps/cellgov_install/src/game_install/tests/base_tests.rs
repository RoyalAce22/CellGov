//! Base-install rejection paths that fire before the EBOOT NPD parse
//! (synthetic fixtures cannot produce a decryptable EBOOT, so the
//! happy path is covered by the presence-gated real-dump parity test
//! in `tests/parity_pkg.rs`).

#![cfg_attr(not(feature = "decrypt"), allow(unused_imports, dead_code))]

use super::*;
use crate::scratch_dir::scratch;
use crate::test_support::{build_iso, build_npdrm_eboot_header, build_param_sfo, IsoNode};
#[cfg(feature = "decrypt")]
use crate::test_support::{build_pkg, pkg_file, PkgItem};

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
    // The install leaves a lock file under `.cellgov`, so the assertion
    // names the records directory alone.
    assert!(
        !StoreLayout::new(&vfs).installs_dir().exists(),
        "no record for a failed install"
    );
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
    assert!(!rap_consumed(None)); // APP-keyed, no NPD header
    assert!(!rap_consumed(Some(NpdLicense::Free))); // free-klicensee fallback
    assert!(rap_consumed(Some(NpdLicense::Network)));
    assert!(rap_consumed(Some(NpdLicense::Local)));
}

/// The staged-RAP plan for `license` with `rap` supplied, driven
/// through the same license gate `install_pkg` uses.
fn plan_for(license: Option<NpdLicense>, rap: Option<&[u8]>) -> Option<StagedRap> {
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
    let plan = plan_for(Some(NpdLicense::Free), Some(&[0u8; 16]));
    assert!(plan.is_none(), "free license plans no staged RAP");
}

#[test]
fn an_app_keyed_title_plans_no_rap_even_when_one_is_supplied() {
    // No NPD header at all: same drop, via the `None` arm of the gate.
    assert!(plan_for(None, Some(&[0u8; 16])).is_none());
}

#[test]
fn a_network_license_plans_a_rap_under_the_staging_root() {
    let sr = plan_for(Some(NpdLicense::Network), Some(&[0u8; 16]))
        .expect("network license plans a staged RAP");
    assert_eq!(sr.staged_path, Path::new("s").join("rap").join("X.rap"));
    assert_eq!(sr.final_path, Path::new("e").join("X.rap"));
}

#[test]
fn a_local_license_plans_a_rap_under_the_staging_root() {
    let sr = plan_for(Some(NpdLicense::Local), Some(&[0u8; 16]))
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
    let _ = plan_for(Some(NpdLicense::Network), None);
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

/// `finished` means the install completed; a renderer draws its 100%
/// frame on it. A pre-commit fault must therefore leave it unset, and
/// the phase trail must stop at the phase that faulted.
#[cfg(feature = "decrypt")]
#[test]
fn a_pre_commit_fault_never_reports_finished() {
    use crate::test_support::{codes, RecordingReporter};
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
            ..Default::default()
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
    assert!(!reporter.finished(), "a faulted install reported finished");
}
