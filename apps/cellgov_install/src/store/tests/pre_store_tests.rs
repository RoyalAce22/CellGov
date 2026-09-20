use super::*;

use crate::scratch_dir::{scratch, ScratchDir};

/// Placeholder identity: these cases build every tree by hand and name
/// no installed content.
const SYNTHETIC_TITLE_ID: &str = "TEST00000";

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .unwrap_or_else(|e| panic!("create {}: {e}", parent.display()));
    }
    std::fs::write(path, text).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}

/// A root that holds the pre-store firmware mount and nothing else.
fn pre_store_firmware_root() -> ScratchDir {
    let dir = scratch();
    write(
        &dir.join("dev_flash").join("firmware.toml"),
        "format_version = 1\n",
    );
    dir
}

/// A root that holds one flat install record, where the store files
/// none.
fn pre_store_record_root() -> ScratchDir {
    let dir = scratch();
    write(
        &StoreLayout::new(&*dir)
            .installs_dir()
            .join(format!("{SYNTHETIC_TITLE_ID}.install.toml")),
        "format_version = 2\n",
    );
    dir
}

fn refusal(root: &Path) -> PreStoreError {
    preflight(root).expect_err("the root holds pre-store residue")
}

/// The residue a refusal names; panics on any other variant.
fn residue_of(e: &PreStoreError) -> &[PreStoreResidue] {
    match e {
        PreStoreError::Residue { residue, .. } => residue,
        other => panic!("expected a residue refusal, got {other:?}"),
    }
}

/// The artifact kinds a refusal names, in order.
fn kinds(e: &PreStoreError) -> Vec<PreStoreArtifact> {
    residue_of(e).iter().map(|r| r.artifact).collect()
}

#[test]
fn an_empty_root_is_not_the_pre_store_layout() {
    let dir = scratch();
    assert!(preflight(&dir).is_ok());
}

#[test]
fn a_store_root_with_records_filed_by_kind_is_not_the_pre_store_layout() {
    let dir = scratch();
    let installs = StoreLayout::new(&*dir).installs_dir();
    write(&installs.join("firmware").join("4.91.install.toml"), "");
    write(
        &installs
            .join("titles")
            .join(SYNTHETIC_TITLE_ID)
            .join("base.install.toml"),
        "",
    );
    assert!(preflight(&dir).is_ok());
}

#[test]
fn a_guest_visible_title_tree_is_not_pre_store_residue() {
    let dir = scratch();
    write(
        &dir.join("dev_hdd0")
            .join("game")
            .join(SYNTHETIC_TITLE_ID)
            .join("USRDIR")
            .join("EBOOT.BIN"),
        "",
    );
    write(&dir.join("dev_bdvd").join(SYNTHETIC_TITLE_ID).join("x"), "");
    assert!(preflight(&dir).is_ok());
}

#[test]
fn the_pre_store_firmware_mount_is_named_with_the_command_that_rebuilds_it() {
    let dir = pre_store_firmware_root();
    let e = refusal(&dir);
    assert_eq!(kinds(&e), vec![PreStoreArtifact::FirmwareMount]);
    let msg = e.to_string();
    // The message names the mount. An operator who removes only the
    // manifest keeps the tree, and no later run detects that root.
    assert!(msg.contains("dev_flash"), "{msg}");
    assert!(!msg.contains("firmware.toml"), "{msg}");
    assert!(msg.contains("cellgov firmware install"), "{msg}");
    assert!(msg.contains("no migration path"), "{msg}");
}

/// An install that stops before it writes the manifest leaves the mount
/// alone.
#[test]
fn a_pre_store_mount_with_no_manifest_in_it_is_still_refused() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("dev_flash").join("sys")).expect("create the pre-store mount");
    assert_eq!(kinds(&refusal(&dir)), vec![PreStoreArtifact::FirmwareMount]);
}

/// LV2 publishes three flash mount points, and the pre-store install
/// wrote all three at the root.
#[test]
fn every_pre_store_flash_mount_is_named_not_only_the_first() {
    let dir = scratch();
    for mount in ["dev_flash", "dev_flash2", "dev_flash3"] {
        std::fs::create_dir_all(dir.join(mount)).expect("create the pre-store mount");
    }
    let e = refusal(&dir);
    assert_eq!(residue_of(&e).len(), 3);
    let msg = e.to_string();
    for mount in ["dev_flash", "dev_flash2", "dev_flash3"] {
        assert!(msg.contains(mount), "{mount} missing from {msg}");
    }
}

#[test]
fn a_flat_install_record_is_named_with_the_command_that_rebuilds_it() {
    let dir = pre_store_record_root();
    let e = refusal(&dir);
    assert_eq!(kinds(&e), vec![PreStoreArtifact::FlatInstallRecord]);
    let msg = e.to_string();
    assert!(msg.contains(SYNTHETIC_TITLE_ID), "{msg}");
    assert!(msg.contains("cellgov title install"), "{msg}");
}

#[test]
fn one_refusal_names_every_residue_and_both_rebuild_commands() {
    let dir = scratch();
    write(&dir.join("dev_flash").join("firmware.toml"), "");
    write(
        &StoreLayout::new(&*dir)
            .installs_dir()
            .join(format!("{SYNTHETIC_TITLE_ID}.install.toml")),
        "",
    );
    let e = refusal(&dir);
    assert_eq!(
        kinds(&e),
        vec![
            PreStoreArtifact::FirmwareMount,
            PreStoreArtifact::FlatInstallRecord
        ]
    );
    let msg = e.to_string();
    assert!(msg.contains("cellgov firmware install"), "{msg}");
    assert!(msg.contains("cellgov title install"), "{msg}");
    assert!(
        msg.find("cellgov firmware install") < msg.find("cellgov title install"),
        "{msg}"
    );
}

/// An install that runs after the store landed files its record by
/// kind, and leaves the older record where it was. Both layouts then
/// sit in one records directory.
#[test]
fn a_flat_record_beside_records_the_store_filed_by_kind_is_still_refused() {
    let dir = scratch();
    let installs = StoreLayout::new(&*dir).installs_dir();
    write(&installs.join("firmware").join("4.91.install.toml"), "");
    write(
        &installs
            .join("titles")
            .join(SYNTHETIC_TITLE_ID)
            .join("base.install.toml"),
        "",
    );
    write(
        &installs.join(format!("{SYNTHETIC_TITLE_ID}.install.toml")),
        "format_version = 2\n",
    );
    let e = refusal(&dir);
    assert_eq!(kinds(&e), vec![PreStoreArtifact::FlatInstallRecord]);
}

#[test]
fn the_rebuild_commands_are_listed_once_however_many_files_call_for_them() {
    let dir = scratch();
    let installs = StoreLayout::new(&*dir).installs_dir();
    for id in ["TEST00000", "TEST00001"] {
        write(&installs.join(format!("{id}.install.toml")), "");
    }
    let e = refusal(&dir);
    assert_eq!(residue_of(&e).len(), 2);
    let msg = e.to_string();
    assert_eq!(msg.matches("cellgov title install").count(), 1, "{msg}");
}

#[test]
fn each_residue_is_named_relative_to_the_root_the_message_already_states() {
    let dir = pre_store_firmware_root();
    let msg = refusal(&dir).to_string();
    assert_eq!(msg.matches(&dir.display().to_string()).count(), 1, "{msg}");
}

/// The walk is ordered by name, so two runs over one root produce one
/// refusal string.
#[test]
fn the_residue_list_does_not_inherit_the_host_enumeration_order() {
    let dir = scratch();
    let installs = StoreLayout::new(&*dir).installs_dir();
    for id in ["TEST00002", "TEST00000", "TEST00001"] {
        write(&installs.join(format!("{id}.install.toml")), "");
    }
    let e = refusal(&dir);
    let paths: Vec<&PathBuf> = residue_of(&e).iter().map(|r| &r.path).collect();
    // Count first: the order check is vacuous over a list the walk
    // dropped entries from.
    assert_eq!(paths.len(), 3);
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted);
}

/// A directory named like a record is the store's own `firmware/` or
/// `titles/` layer under a different name. Only a file is residue.
#[test]
fn a_directory_under_the_records_root_is_not_a_flat_record() {
    let dir = scratch();
    std::fs::create_dir_all(
        StoreLayout::new(&*dir)
            .installs_dir()
            .join("titles.install.toml"),
    )
    .expect("create the decoy directory");
    assert!(preflight(&dir).is_ok());
}
