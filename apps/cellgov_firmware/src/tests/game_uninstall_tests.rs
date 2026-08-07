//! Record-driven uninstall: round-trip teardown, verify gate, disc vs
//! HDD target resolution, idempotency, and stale-tombstone sweeping.
//! Synthetic fixtures only (uninstall needs no decryptable EBOOT --
//! the tree + record are hand-written).

use super::*;
use crate::game_install::{
    sha256_of, InstallRecord, RapRecord, SourceRecord, TitleRecord, INSTALL_RECORD_FORMAT_VERSION,
};
use crate::test_support::scratch;

/// Hand-write a game tree + RAP + install record under `out`/`installs`
/// (uninstall is record-driven and needs no decryptable EBOOT).
fn stage_synthetic_install(
    out: &Path,
    installs: &Path,
    title_id: &str,
    is_disc: bool,
    files: &[(&str, &[u8])],
    rap: Option<(&str, &[u8])>,
) {
    let game_dir = if is_disc {
        out.join("dev_bdvd").join(title_id)
    } else {
        out.join("dev_hdd0").join("game").join(title_id)
    };
    let mut filemap = std::collections::BTreeMap::new();
    for (rel, bytes) in files {
        let p = game_dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        filemap.insert((*rel).to_string(), sha256_of(bytes));
    }
    let rap_rec = rap.map(|(name, bytes)| {
        let exdata = out.join("dev_hdd0/home/00000001/exdata");
        std::fs::create_dir_all(&exdata).unwrap();
        std::fs::write(exdata.join(name), bytes).unwrap();
        RapRecord {
            filename: name.to_string(),
            sha256: sha256_of(bytes),
        }
    });
    let record = InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        source: SourceRecord {
            kind: if is_disc { "iso" } else { "pkg" }.to_string(),
            sha256: sha256_of(b"src"),
        },
        title: TitleRecord {
            title_id: title_id.to_string(),
            content_id: title_id.to_string(),
            category: if is_disc { "DG" } else { "HG" }.to_string(),
            title: "T".to_string(),
            app_version: "01.00".to_string(),
            distribution: if is_disc { "disc-iso" } else { "psn-hdd" }.to_string(),
        },
        files: filemap,
        rap: rap_rec,
    };
    std::fs::create_dir_all(installs).unwrap();
    std::fs::write(
        installs.join(format!("{title_id}.install.toml")),
        toml::to_string(&record).unwrap(),
    )
    .unwrap();
}

const NO_VERIFY: UninstallOptions = UninstallOptions {
    verify: false,
    keep_rap: false,
    force: false,
};

#[test]
fn uninstall_round_trip_removes_tree_rap_record() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(
        &vfs,
        &installs,
        "NPUA80001",
        false,
        &[("USRDIR/EBOOT.BIN", b"eboot")],
        Some(("UP9000-NPUA80001_00-X.rap", &[7u8; 16])),
    );

    let outcome = uninstall("NPUA80001", &vfs, &installs, NO_VERIFY).expect("uninstall");
    assert!(!vfs.join("dev_hdd0/game/NPUA80001").exists());
    assert!(!vfs
        .join("dev_hdd0/home/00000001/exdata/UP9000-NPUA80001_00-X.rap")
        .exists());
    assert!(!installs.join("NPUA80001.install.toml").exists());
    assert!(!vfs.join("dev_hdd0/game/.uninstalling-NPUA80001").exists());
    assert_eq!(outcome.files_verified, None);
    assert!(outcome.rap_removed.is_some());
}

#[test]
fn uninstall_no_record_errors() {
    let out = scratch();
    let err = uninstall(
        "NPUA99999",
        &out.join("vfs"),
        &out.join("installs"),
        NO_VERIFY,
    )
    .unwrap_err();
    assert!(matches!(err, GameUninstallError::NoRecord { title_id } if title_id == "NPUA99999"));
}

#[test]
fn uninstall_verify_detects_modified_tree_and_force_overrides() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(
        &vfs,
        &installs,
        "NPUA80001",
        false,
        &[("USRDIR/EBOOT.BIN", b"original")],
        None,
    );
    // Tamper with a live file after recording.
    std::fs::write(
        vfs.join("dev_hdd0/game/NPUA80001/USRDIR/EBOOT.BIN"),
        b"tampered",
    )
    .unwrap();

    let verify = UninstallOptions {
        verify: true,
        keep_rap: false,
        force: false,
    };
    let err = uninstall("NPUA80001", &vfs, &installs, verify).unwrap_err();
    assert!(
        matches!(err, GameUninstallError::TreeModified { .. }),
        "verify must catch the tamper, got {err:?}"
    );
    // The tree is untouched by a failed verify.
    assert!(vfs.join("dev_hdd0/game/NPUA80001").exists());

    // force overrides the gate.
    let forced = UninstallOptions {
        verify: true,
        keep_rap: false,
        force: true,
    };
    let outcome = uninstall("NPUA80001", &vfs, &installs, forced).expect("force uninstall");
    assert!(!vfs.join("dev_hdd0/game/NPUA80001").exists());
    assert_eq!(outcome.files_verified, Some(0)); // the one file mismatched
}

#[test]
fn uninstall_tree_already_gone_still_succeeds() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(
        &vfs,
        &installs,
        "NPUA80001",
        false,
        &[("USRDIR/EBOOT.BIN", b"eboot")],
        Some(("R.rap", &[1u8; 16])),
    );
    // Tree vanished out from under the record.
    std::fs::remove_dir_all(vfs.join("dev_hdd0/game/NPUA80001")).unwrap();

    uninstall("NPUA80001", &vfs, &installs, NO_VERIFY).expect("idempotent uninstall");
    assert!(!installs.join("NPUA80001.install.toml").exists());
    assert!(!vfs.join("dev_hdd0/home/00000001/exdata/R.rap").exists());
}

#[test]
fn uninstall_twice_second_is_no_record() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(&vfs, &installs, "NPUA80001", false, &[("X", b"x")], None);
    uninstall("NPUA80001", &vfs, &installs, NO_VERIFY).expect("first uninstall");
    let err = uninstall("NPUA80001", &vfs, &installs, NO_VERIFY).unwrap_err();
    assert!(matches!(err, GameUninstallError::NoRecord { .. }));
}

#[test]
fn uninstall_disc_touches_no_exdata() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    // A foreign RAP from another title must survive a disc uninstall.
    let exdata = vfs.join("dev_hdd0/home/00000001/exdata");
    std::fs::create_dir_all(&exdata).unwrap();
    std::fs::write(exdata.join("FOREIGN.rap"), [9u8; 16]).unwrap();
    stage_synthetic_install(
        &vfs,
        &installs,
        "BCES00664",
        true,
        &[("PS3_GAME/USRDIR/EBOOT.BIN", b"eboot")],
        None,
    );

    let outcome = uninstall("BCES00664", &vfs, &installs, NO_VERIFY).expect("disc uninstall");
    assert!(!vfs.join("dev_bdvd/BCES00664").exists());
    assert!(
        exdata.join("FOREIGN.rap").exists(),
        "disc uninstall left exdata alone"
    );
    assert!(outcome.rap_removed.is_none());
}

#[test]
fn uninstall_clears_stale_tombstone_on_entry() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(&vfs, &installs, "NPUA80001", false, &[("X", b"x")], None);
    // A leftover tombstone from a prior interrupted uninstall.
    let tombstone = vfs.join("dev_hdd0/game/.uninstalling-NPUA80001");
    std::fs::create_dir_all(&tombstone).unwrap();
    std::fs::write(tombstone.join("junk"), b"junk").unwrap();

    uninstall("NPUA80001", &vfs, &installs, NO_VERIFY).expect("uninstall over stale tombstone");
    assert!(!tombstone.exists(), "stale tombstone swept");
    assert!(!installs.join("NPUA80001.install.toml").exists());
}
