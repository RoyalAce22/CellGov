//! Every entry point that writes the store claims its artifact first:
//! what a held claim refuses, what it leaves untouched, and what it
//! still allows.

#![cfg_attr(not(feature = "decrypt"), allow(unused_imports, dead_code))]

use std::path::{Path, PathBuf};

use crate::game_install::sha256_of;
use crate::game_uninstall::{self, UninstallOptions, UninstallScope};
use crate::scratch_dir::scratch;
use crate::store::layout::{Artifact, StoreLayout, TitleId, TitleTree, VersionKey};
use crate::store::lock::{lock_artifact, lock_firmware_staging};
use crate::store::record::{
    ArtifactRecord, InstallRecord, SourceRecord, TitleRecord, INSTALL_RECORD_FORMAT_VERSION,
};
use crate::{firmware_uninstall, game_install};

#[cfg(feature = "decrypt")]
use crate::firmware_install;
#[cfg(feature = "decrypt")]
use crate::keys::KeyVault;
#[cfg(feature = "decrypt")]
use crate::test_support::{
    build_iso, build_npdrm_eboot_header, build_param_sfo, build_pkg, build_pup, build_tar,
    pkg_file, IsoNode,
};
#[cfg(feature = "decrypt")]
use cellgov_ps3_abi::format::pup::ENTRY_ID_UPDATE_FILES;

/// Placeholder identities: every fixture here is hand-built and names
/// no installed content.
const TITLE_ID: &str = "TEST00000";
const CONTENT_ID: &str = "UP0000-TEST00000_00-SYNTHETIC00000";
const UPDATE_VERSION: &str = "02.51";
const FIRMWARE_VERSION: &str = "4.91";

#[cfg(feature = "decrypt")]
const KLIC: [u8; 16] = [
    0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xA0, 0xB0, 0xC0, 0xD0, 0xE0, 0xF0, 0x01,
];

#[cfg(feature = "decrypt")]
fn keys() -> KeyVault {
    crate::test_support::synthetic_vault()
}

fn base_artifact(title_id: &str) -> Artifact {
    Artifact::TitleBase {
        title_id: TitleId::new(title_id).expect("synthetic title id"),
    }
}

fn update_artifact(title_id: &str, version: &str) -> Artifact {
    Artifact::TitleUpdate {
        title_id: TitleId::new(title_id).expect("synthetic title id"),
        version: VersionKey::new(version).expect("synthetic version"),
    }
}

fn firmware_artifact(version: &str) -> Artifact {
    Artifact::Firmware {
        version: VersionKey::new(version).expect("synthetic version"),
    }
}

fn write(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
    }
    std::fs::write(path, bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// Write the record for `artifact`, each of `files` at the hash the
/// staging helpers below write, so a verifying removal matches.
fn write_record(vfs: &Path, artifact: &Artifact, tree: &Path, files: &[&str]) {
    let layout = StoreLayout::new(vfs);
    let record = InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: artifact.kind(),
            version: match artifact {
                Artifact::TitleBase { .. } => "01.00".to_string(),
                Artifact::TitleUpdate { version, .. } | Artifact::Firmware { version } => {
                    version.as_str().to_string()
                }
            },
            store_path: layout.store_path_of(tree).expect("under the vfs root"),
        },
        source: SourceRecord::local("pkg", sha256_of(b"source")),
        title: match artifact {
            Artifact::Firmware { .. } => None,
            _ => Some(TitleRecord {
                title_id: TITLE_ID.to_string(),
                content_id: TITLE_ID.to_string(),
                category: "HG".to_string(),
                title: "Synthetic".to_string(),
                distribution: "psn-hdd".to_string(),
                system_ver: None,
                shipped_firmware: None,
            }),
        },
        files: files
            .iter()
            .map(|f| ((*f).to_string(), sha256_of(b"file")))
            .collect(),
        rap: None,
        core_os: None,
    };
    write(
        &layout.record_path(artifact),
        record.to_toml().expect("serialize the record").as_bytes(),
    );
}

/// A base tree and its record, as an install would leave them.
fn stage_base(vfs: &Path) -> PathBuf {
    let tree = vfs.join("dev_hdd0").join("game").join(TITLE_ID);
    write(&tree.join("USRDIR/EBOOT.BIN"), b"file");
    write_record(vfs, &base_artifact(TITLE_ID), &tree, &["USRDIR/EBOOT.BIN"]);
    tree
}

/// One update entry and its record over an installed base.
fn stage_update(vfs: &Path, version: &str) -> PathBuf {
    let artifact = update_artifact(TITLE_ID, version);
    let tree = StoreLayout::new(vfs).entry_dir(&artifact);
    let rel = format!("{}/USRDIR/EBOOT.BIN", TitleTree::Game.dir_name());
    write(&tree.join(&rel), b"file");
    write_record(vfs, &artifact, &tree, &[rel.as_str()]);
    tree
}

fn stage_firmware(vfs: &Path, version: &str) -> PathBuf {
    let artifact = firmware_artifact(version);
    let tree = StoreLayout::new(vfs).entry_dir(&artifact);
    write(&tree.join("dev_flash/vsh/etc/version.txt"), b"file");
    write_record(vfs, &artifact, &tree, &[]);
    tree
}

const NO_VERIFY: UninstallOptions = UninstallOptions {
    verify: false,
    keep_rap: false,
    force: false,
};

/// A synthetic HG PKG whose EBOOT carries a free-license NPD header, so
/// the install reaches the claim without needing a RAP.
#[cfg(feature = "decrypt")]
fn base_pkg() -> Vec<u8> {
    let sfo = build_param_sfo(&[("TITLE_ID", TITLE_ID), ("CATEGORY", "HG")]);
    let eboot = build_npdrm_eboot_header(3, CONTENT_ID);
    build_pkg(
        &keys(),
        &KLIC,
        CONTENT_ID,
        &[
            pkg_file("PARAM.SFO", 3, &sfo),
            pkg_file("USRDIR/EBOOT.BIN", 1, &eboot),
        ],
    )
}

#[cfg(feature = "decrypt")]
fn update_pkg() -> Vec<u8> {
    let sfo = build_param_sfo(&[
        ("TITLE_ID", TITLE_ID),
        ("CATEGORY", "GD"),
        ("APP_VER", UPDATE_VERSION),
    ]);
    build_pkg(
        &keys(),
        &KLIC,
        CONTENT_ID,
        &[
            pkg_file("PARAM.SFO", 3, &sfo),
            pkg_file("USRDIR/EBOOT.BIN", 1, b"patched"),
        ],
    )
}

/// A PUP carrying one dev_flash package. The package is not a real SCE
/// container, so an install that passes the claim fails later in
/// extraction.
#[cfg(feature = "decrypt")]
fn firmware_pup() -> Vec<u8> {
    let outer = build_tar(&[("dev_flash_0.tar", b"not a real SCE package")]);
    build_pup(
        &keys(),
        0x0004_9100_0000_0000,
        &[(ENTRY_ID_UPDATE_FILES, &outer)],
    )
}

#[cfg(feature = "decrypt")]
fn install_base(vfs: &Path) -> Result<(), game_install::GameInstallError> {
    game_install::install_pkg(
        &base_pkg(),
        None,
        &keys(),
        vfs,
        game_install::InstallOptions::default(),
    )
    .map(|_| ())
}

#[cfg(feature = "decrypt")]
#[test]
fn a_held_base_refuses_a_second_install_and_stages_nothing() {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);

    let _held = lock_artifact(&layout, &base_artifact(TITLE_ID)).expect("claim the base");
    let err = install_base(&vfs).expect_err("a claimed base refuses");

    assert!(
        matches!(err, game_install::GameInstallError::Locked(_)),
        "{err}"
    );
    assert!(
        !vfs.join("dev_hdd0").exists(),
        "the refusal came before anything was staged"
    );
}

/// The control for [`a_held_base_refuses_a_second_install_and_stages_nothing`]:
/// with the claim free, the same PKG reaches the decrypt-proof, so the
/// claim causes the refusal above.
#[cfg(feature = "decrypt")]
#[test]
fn an_unheld_base_install_runs_past_the_claim() {
    let out = scratch();
    let err = install_base(&out.join("vfs")).expect_err("the synthetic EBOOT does not decrypt");

    assert!(
        matches!(err, game_install::GameInstallError::DecryptProof(_)),
        "{err}"
    );
}

/// A disc image of the same title. Its EBOOT opens with the SCE magic,
/// so it passes the still-encrypted gate and reaches the claim. It is no
/// SELF, so an unclaimed run stops at the decrypt-proof.
#[cfg(feature = "decrypt")]
fn disc_image() -> Vec<u8> {
    let mut eboot = cellgov_ps3_abi::format::sce::SCE_MAGIC.to_vec();
    eboot.extend_from_slice(b" not a SELF");
    build_iso(vec![IsoNode::Dir(
        "PS3_GAME",
        vec![
            IsoNode::File(
                "PARAM.SFO",
                build_param_sfo(&[("TITLE_ID", TITLE_ID), ("CATEGORY", "DG")]),
            ),
            IsoNode::Dir("USRDIR", vec![IsoNode::File("EBOOT.BIN", eboot)]),
        ],
    )])
}

/// A disc base and an HDD base of one title share one record, so they
/// share one claim, even though they commit to different mounts.
#[cfg(feature = "decrypt")]
#[test]
fn a_held_base_refuses_a_disc_install_of_the_same_title() {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);

    let _held = lock_artifact(&layout, &base_artifact(TITLE_ID)).expect("claim the base");
    let err = game_install::install_iso(
        &disc_image(),
        &keys(),
        &vfs,
        game_install::InstallOptions::default(),
    )
    .expect_err("a claimed base refuses the disc path too");

    assert!(
        matches!(err, game_install::GameInstallError::Locked(_)),
        "{err}"
    );
    assert!(
        !vfs.join("dev_bdvd").exists(),
        "the refusal came before anything was staged"
    );
}

/// The control for [`a_held_base_refuses_a_disc_install_of_the_same_title`].
#[cfg(feature = "decrypt")]
#[test]
fn an_unheld_disc_install_runs_past_the_claim() {
    let out = scratch();
    let err = game_install::install_iso(
        &disc_image(),
        &keys(),
        &out.join("vfs"),
        game_install::InstallOptions::default(),
    )
    .expect_err("the synthetic disc EBOOT does not decrypt");

    assert!(
        matches!(err, game_install::GameInstallError::DecryptProof(_)),
        "{err}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_failed_install_leaves_the_base_claimable() {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);
    let artifact = base_artifact(TITLE_ID);
    install_base(&vfs).expect_err("the synthetic EBOOT does not decrypt");

    // The lock file shows the install took a claim at all; a claimable
    // path proves nothing on its own.
    assert!(
        layout.lock_path(&artifact).is_file(),
        "the install claimed the base on its way in"
    );
    let _free = lock_artifact(&layout, &artifact).expect("a failed install releases its claim");
}

#[cfg(feature = "decrypt")]
#[test]
fn three_held_titles_do_not_hold_a_fourth() {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);

    let _held: Vec<_> = ["TEST00001", "TEST00002", "TEST00003"]
        .iter()
        .map(|id| lock_artifact(&layout, &base_artifact(id)).expect("claim a sibling title"))
        .collect();

    // Reaching the decrypt-proof means the claim was granted: the store
    // is not one lock.
    let err = install_base(&vfs).expect_err("the synthetic EBOOT does not decrypt");
    assert!(
        matches!(err, game_install::GameInstallError::DecryptProof(_)),
        "{err}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_held_update_version_refuses_a_second_install_and_lets_go() {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);
    let artifact = update_artifact(TITLE_ID, UPDATE_VERSION);

    let held = lock_artifact(&layout, &artifact).expect("claim the update");
    let err = game_install::install_update_pkg(
        &update_pkg(),
        &keys(),
        &vfs,
        game_install::InstallOptions::default(),
    )
    .expect_err("a claimed update refuses");
    assert!(
        matches!(err, game_install::GameInstallError::Locked(_)),
        "{err}"
    );
    assert!(
        !layout.entry_dir(&artifact).exists(),
        "the refusal came before anything was staged"
    );

    drop(held);
    game_install::install_update_pkg(
        &update_pkg(),
        &keys(),
        &vfs,
        game_install::InstallOptions::default(),
    )
    .expect("the same PKG installs once the claim is free");
}

#[cfg(feature = "decrypt")]
#[test]
fn a_held_base_does_not_hold_its_own_update() {
    let out = scratch();
    let vfs = out.join("vfs");

    let _held =
        lock_artifact(&StoreLayout::new(&vfs), &base_artifact(TITLE_ID)).expect("claim the base");
    game_install::install_update_pkg(
        &update_pkg(),
        &keys(),
        &vfs,
        game_install::InstallOptions::default(),
    )
    .expect("an update version is claimed on its own key");
}

#[cfg(feature = "decrypt")]
#[test]
fn a_held_firmware_staging_directory_refuses_an_install() {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);

    let _held = lock_firmware_staging(&layout).expect("claim the staging directory");
    let err = firmware_install::install_pup(&firmware_pup(), &keys(), &vfs, false, &())
        .expect_err("a claimed staging directory refuses");

    assert!(
        matches!(err, firmware_install::FirmwareInstallError::Locked(_)),
        "{err}"
    );
    assert!(
        !layout.firmware_staging_dir().exists(),
        "the refusal came before the staging sweep"
    );
}

/// The control for
/// [`a_held_firmware_staging_directory_refuses_an_install`]: with the
/// claim free, the same PUP runs on into extraction.
#[cfg(feature = "decrypt")]
#[test]
fn an_unheld_firmware_install_runs_past_the_claim() {
    let out = scratch();
    let err = firmware_install::install_pup(&firmware_pup(), &keys(), &out.join("vfs"), false, &())
        .expect_err("the synthetic package is no SCE container");

    assert!(
        matches!(
            err,
            firmware_install::FirmwareInstallError::PartialInstall { .. }
        ),
        "{err}"
    );
}

#[test]
fn a_held_base_refuses_an_uninstall_and_removes_nothing() {
    let out = scratch();
    let vfs = out.join("vfs");
    let tree = stage_base(&vfs);
    let layout = StoreLayout::new(&vfs);
    let artifact = base_artifact(TITLE_ID);

    let _held = lock_artifact(&layout, &artifact).expect("claim the base");
    let err = game_uninstall::uninstall(TITLE_ID, &vfs, &UninstallScope::Base, NO_VERIFY)
        .expect_err("a claimed base refuses");

    assert!(
        matches!(err, game_uninstall::GameUninstallError::Locked(_)),
        "{err}"
    );
    assert!(tree.join("USRDIR/EBOOT.BIN").is_file());
    assert!(layout.record_path(&artifact).is_file());
}

#[test]
fn one_held_update_refuses_the_whole_title_and_leaves_the_base() {
    let out = scratch();
    let vfs = out.join("vfs");
    let base = stage_base(&vfs);
    let update = stage_update(&vfs, UPDATE_VERSION);

    let _held = lock_artifact(
        &StoreLayout::new(&vfs),
        &update_artifact(TITLE_ID, UPDATE_VERSION),
    )
    .expect("claim one update");
    let err = game_uninstall::uninstall(TITLE_ID, &vfs, &UninstallScope::All, NO_VERIFY)
        .expect_err("one claimed entry refuses the scope");

    assert!(
        matches!(err, game_uninstall::GameUninstallError::Locked(_)),
        "{err}"
    );
    assert!(base.join("USRDIR/EBOOT.BIN").is_file(), "all or none");
    assert!(update.exists());
}

#[test]
fn an_uninstall_releases_its_claims() {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);
    let artifact = base_artifact(TITLE_ID);
    stage_base(&vfs);

    game_uninstall::uninstall(TITLE_ID, &vfs, &UninstallScope::Base, NO_VERIFY)
        .expect("uninstall the base");
    assert!(
        layout.lock_path(&artifact).is_file(),
        "the removal claimed the base before it took anything"
    );
    let _free = lock_artifact(&layout, &artifact).expect("the removal released its claim");
}

/// The window `recheck_under_claim` closes. A re-install onto another
/// mount moves the tree while the record path stays put, so the plan
/// and the record disagree.
#[test]
fn a_base_record_that_moved_between_the_plan_and_the_claim_is_refused() {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);
    let artifact = base_artifact(TITLE_ID);
    let planned_tree = stage_base(&vfs);

    let plan =
        game_uninstall::plan(TITLE_ID, &vfs, &UninstallScope::Base).expect("plan the base removal");

    // A concurrent install lands the same title on the disc mount and
    // rewrites the record; the plan still names the HDD tree.
    let moved_tree = vfs.join("dev_bdvd").join(TITLE_ID);
    write(&moved_tree.join("USRDIR/EBOOT.BIN"), b"file");
    write_record(&vfs, &artifact, &moved_tree, &["USRDIR/EBOOT.BIN"]);

    let err = game_uninstall::execute(&plan, NO_VERIFY).expect_err("the record moved");
    assert!(
        matches!(
            err,
            game_uninstall::GameUninstallError::RecordMovedSincePlan { .. }
        ),
        "{err}"
    );
    assert!(
        planned_tree.join("USRDIR/EBOOT.BIN").is_file(),
        "the tree the plan named is still there"
    );
    assert!(
        moved_tree.join("USRDIR/EBOOT.BIN").is_file(),
        "the tree the record names now is untouched"
    );
    assert!(
        layout.record_path(&artifact).is_file(),
        "the record the other writer left is not this removal's to take"
    );
}

/// The control for
/// [`a_base_record_that_moved_between_the_plan_and_the_claim_is_refused`]:
/// an unmoved record still removes, so the re-read causes the refusal
/// above rather than the split plan / execute call.
#[test]
fn a_base_record_that_stayed_put_between_the_plan_and_the_claim_removes() {
    let out = scratch();
    let vfs = out.join("vfs");
    let tree = stage_base(&vfs);

    let plan =
        game_uninstall::plan(TITLE_ID, &vfs, &UninstallScope::Base).expect("plan the base removal");
    let outcome = game_uninstall::execute(&plan, NO_VERIFY).expect("nothing moved");

    assert_eq!(outcome.removed.len(), 1);
    assert!(!tree.exists());
}

#[test]
fn a_held_firmware_version_refuses_an_uninstall_and_removes_nothing() {
    let out = scratch();
    let vfs = out.join("vfs");
    let tree = stage_firmware(&vfs, FIRMWARE_VERSION);
    let layout = StoreLayout::new(&vfs);
    let artifact = firmware_artifact(FIRMWARE_VERSION);

    let _held = lock_artifact(&layout, &artifact).expect("claim the version");
    let err = firmware_uninstall::uninstall(FIRMWARE_VERSION, &vfs)
        .expect_err("a claimed version refuses");

    assert!(
        matches!(err, firmware_uninstall::FirmwareUninstallError::Locked(_)),
        "{err}"
    );
    assert!(tree.join("dev_flash/vsh/etc/version.txt").is_file());
    assert!(layout.record_path(&artifact).is_file());
}

#[test]
fn one_held_firmware_version_does_not_hold_another() {
    let out = scratch();
    let vfs = out.join("vfs");
    stage_firmware(&vfs, FIRMWARE_VERSION);
    stage_firmware(&vfs, "4.90");

    let _held = lock_artifact(
        &StoreLayout::new(&vfs),
        &firmware_artifact(FIRMWARE_VERSION),
    )
    .expect("claim one version");
    firmware_uninstall::uninstall("4.90", &vfs).expect("another version is claimed on its own key");
}
