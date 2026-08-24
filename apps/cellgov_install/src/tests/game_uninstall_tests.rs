//! Record-driven uninstall: round-trip teardown, verify gate, disc vs
//! HDD target resolution, idempotency, and stale-tombstone sweeping.
//! Synthetic fixtures only (uninstall needs no decryptable EBOOT --
//! the tree + record are hand-written).

use super::*;
use crate::game_install::{
    sha256_of, InstallRecord, RapRecord, SourceRecord, TitleRecord, INSTALL_RECORD_FORMAT_VERSION,
};
use crate::scratch_dir::scratch;

/// Hand-write a game tree + RAP + install record under `out`/`installs`.
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

const VERIFY: UninstallOptions = UninstallOptions {
    verify: true,
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
    assert_eq!(
        outcome.rap_removed.as_deref(),
        Some(
            vfs.join("dev_hdd0/home/00000001/exdata/UP9000-NPUA80001_00-X.rap")
                .as_path()
        ),
        "the outcome names the RAP it removed, not just that it removed one"
    );
    assert_eq!(
        outcome.game_dir_removed,
        vfs.join("dev_hdd0/game/NPUA80001")
    );
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
    assert!(vfs.join("dev_hdd0/game/NPUA80001").exists());

    let forced = UninstallOptions {
        verify: true,
        keep_rap: false,
        force: true,
    };
    let outcome = uninstall("NPUA80001", &vfs, &installs, forced).expect("force uninstall");
    assert!(!vfs.join("dev_hdd0/game/NPUA80001").exists());
    assert_eq!(outcome.files_verified, Some(0));
    assert_eq!(
        outcome.files_diverged,
        Some(1),
        "the tamper is counted, not just tolerated"
    );
}

/// An intact RAP counts in `files_verified`, as a diverged one counts
/// in `files_diverged`: the two tallies are drawn from the same set, so
/// counting only the RAP's divergence would let `diverged` exceed it.
#[test]
fn verify_on_an_intact_tree_counts_every_recorded_file_and_the_rap() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(
        &vfs,
        &installs,
        "NPUA80001",
        false,
        &[("USRDIR/EBOOT.BIN", b"eboot"), ("PARAM.SFO", b"sfo")],
        Some(("UP9000-NPUA80001_00-X.rap", &[7u8; 16])),
    );

    let outcome = uninstall("NPUA80001", &vfs, &installs, VERIFY).expect("intact tree verifies");
    // Two recorded files plus the recorded RAP.
    assert_eq!(outcome.files_verified, Some(3));
    assert_eq!(outcome.files_diverged, Some(0));
    assert!(!vfs.join("dev_hdd0/game/NPUA80001").exists());
}

/// Same two files, no RAP in the record: the count drops by exactly the
/// RAP, so the extra unit above is the RAP and not a miscount.
#[test]
fn a_record_with_no_rap_verifies_only_its_files() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(
        &vfs,
        &installs,
        "NPUA80001",
        false,
        &[("USRDIR/EBOOT.BIN", b"eboot"), ("PARAM.SFO", b"sfo")],
        None,
    );

    let outcome = uninstall("NPUA80001", &vfs, &installs, VERIFY).expect("intact tree verifies");
    assert_eq!(outcome.files_verified, Some(2));
    assert_eq!(outcome.files_diverged, Some(0));
}

#[test]
fn verify_detects_a_tampered_rap_and_leaves_it_in_place() {
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
    let rap = vfs.join("dev_hdd0/home/00000001/exdata/UP9000-NPUA80001_00-X.rap");
    std::fs::write(&rap, [8u8; 16]).unwrap();

    let err = uninstall("NPUA80001", &vfs, &installs, VERIFY).unwrap_err();
    assert!(
        matches!(&err, GameUninstallError::TreeModified { path, .. } if path == &rap),
        "verify must name the RAP, got {err:?}"
    );
    assert!(rap.exists(), "a failed verify removes nothing");
    assert!(vfs.join("dev_hdd0/game/NPUA80001").exists());
    assert!(installs.join("NPUA80001.install.toml").exists());
}

#[test]
fn keep_rap_leaves_the_rap_in_exdata() {
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

    let keep = UninstallOptions {
        verify: false,
        keep_rap: true,
        force: false,
    };
    let outcome = uninstall("NPUA80001", &vfs, &installs, keep).expect("uninstall");
    assert!(
        vfs.join("dev_hdd0/home/00000001/exdata/UP9000-NPUA80001_00-X.rap")
            .exists(),
        "keep_rap leaves the RAP for a title that may share it"
    );
    assert!(outcome.rap_removed.is_none());
    assert!(!vfs.join("dev_hdd0/game/NPUA80001").exists());
    assert!(!installs.join("NPUA80001.install.toml").exists());
}

#[test]
fn a_record_that_does_not_parse_is_named_and_removes_nothing() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(&vfs, &installs, "NPUA80001", false, &[("X", b"x")], None);
    let record = installs.join("NPUA80001.install.toml");
    std::fs::write(&record, "format_version = ").unwrap();

    let err = uninstall("NPUA80001", &vfs, &installs, NO_VERIFY).unwrap_err();
    assert!(
        matches!(err, GameUninstallError::RecordParse(_)),
        "an unparseable record is not a NoRecord miss, got {err:?}"
    );
    assert!(record.exists(), "an unreadable record is not deleted");
    assert!(vfs.join("dev_hdd0/game/NPUA80001").exists());
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

const FORCED_VERIFY: UninstallOptions = UninstallOptions {
    verify: true,
    keep_rap: false,
    force: true,
};

/// A container can carry a zero-byte entry, so the record legitimately
/// holds the empty-bytes hash for one. Reading absence as those bytes
/// would let a deleted placeholder pass the gate as intact.
#[test]
fn a_deleted_zero_byte_file_is_a_divergence_not_a_match() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(
        &vfs,
        &installs,
        "NPUA80001",
        false,
        &[("USRDIR/EBOOT.BIN", b"eboot"), ("USRDIR/EMPTY.DAT", b"")],
        None,
    );
    let empty = vfs.join("dev_hdd0/game/NPUA80001/USRDIR/EMPTY.DAT");
    assert!(empty.is_file(), "the record covers a zero-byte entry");
    std::fs::remove_file(&empty).unwrap();

    let err = uninstall("NPUA80001", &vfs, &installs, VERIFY).unwrap_err();
    assert!(
        matches!(&err, GameUninstallError::RecordedFileMissing { path } if path == &empty),
        "an absent recorded file must be named, got {err:?}"
    );
    assert!(vfs.join("dev_hdd0/game/NPUA80001").exists());
}

#[test]
fn force_counts_the_divergences_it_waves_through() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(
        &vfs,
        &installs,
        "NPUA80001",
        false,
        &[
            ("KEPT.DAT", b"kept"),
            ("TAMPERED.DAT", b"original"),
            ("DELETED.DAT", b"gone"),
        ],
        Some(("UP9000-NPUA80001_00-X.rap", &[7u8; 16])),
    );
    let game = vfs.join("dev_hdd0/game/NPUA80001");
    std::fs::write(game.join("TAMPERED.DAT"), b"tampered").unwrap();
    std::fs::remove_file(game.join("DELETED.DAT")).unwrap();
    std::fs::write(
        vfs.join("dev_hdd0/home/00000001/exdata/UP9000-NPUA80001_00-X.rap"),
        [8u8; 16],
    )
    .unwrap();

    let outcome = uninstall("NPUA80001", &vfs, &installs, FORCED_VERIFY).expect("force uninstall");
    assert_eq!(outcome.files_verified, Some(1));
    // One tampered file, one deleted file, one tampered RAP.
    assert_eq!(outcome.files_diverged, Some(3));
}

#[test]
fn a_verify_that_finds_nothing_wrong_reports_zero_divergences() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(&vfs, &installs, "NPUA80001", false, &[("X", b"x")], None);

    let outcome = uninstall("NPUA80001", &vfs, &installs, VERIFY).expect("intact tree");
    assert_eq!(outcome.files_verified, Some(1));
    assert_eq!(outcome.files_diverged, Some(0));
}

#[test]
fn without_verify_neither_witness_is_reported() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(&vfs, &installs, "NPUA80001", false, &[("X", b"x")], None);

    let outcome = uninstall("NPUA80001", &vfs, &installs, NO_VERIFY).expect("uninstall");
    assert_eq!(outcome.files_verified, None);
    assert_eq!(outcome.files_diverged, None);
}

/// The title-id spells a game directory, a record filename and a
/// tombstone, so it takes the same path-component rule the install side
/// applies before it commits under an id.
#[test]
fn an_unsafe_title_id_is_refused_before_any_path_is_built() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    std::fs::create_dir_all(&installs).unwrap();

    for bad in [
        "",
        "..",
        ".staging-NPUA80001",
        "../NPUA80001",
        "a/b",
        "a\\b",
    ] {
        let err = uninstall(bad, &vfs, &installs, NO_VERIFY).expect_err("must be refused");
        assert!(
            matches!(&err, GameUninstallError::UnsafeTitleId { title_id } if title_id == bad),
            "expected UnsafeTitleId for {bad:?}, got {err:?}"
        );
    }
}

#[test]
fn a_rap_that_was_already_gone_is_not_reported_as_removed() {
    let out = scratch();
    let vfs = out.join("vfs");
    let installs = out.join("installs");
    stage_synthetic_install(
        &vfs,
        &installs,
        "NPUA80001",
        false,
        &[("X", b"x")],
        Some(("UP9000-NPUA80001_00-X.rap", &[7u8; 16])),
    );
    let rap = vfs.join("dev_hdd0/home/00000001/exdata/UP9000-NPUA80001_00-X.rap");
    std::fs::remove_file(&rap).unwrap();

    let outcome = uninstall("NPUA80001", &vfs, &installs, NO_VERIFY).expect("uninstall");
    assert_eq!(
        outcome.rap_removed, None,
        "the outcome names what this call took away, not what the record listed"
    );
    assert!(!installs.join("NPUA80001.install.toml").exists());
}
