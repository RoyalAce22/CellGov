//! Where the local PS3 corpus lives on disk.
//!
//! The store keys firmware on version, so the firmware install record
//! names the installed tree. This module reads those records the way
//! the boot path does. A suite that asserts about one firmware library
//! names the version it means.
//!
//! Those paths reach a tree git does not track, so this module
//! declares the corpus features itself. A helper module has no
//! `[[test]]` target to carry `required-features`.
#![cfg(any(feature = "firmware-corpus", feature = "ps3autotests"))]
#![allow(
    dead_code,
    reason = "each integration-test binary compiles this module separately and uses a subset"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use cellgov_install::store::{ArtifactKind, InstallRecord, StoreLayout, DEFAULT_VFS_ROOT};
use cellgov_ps3_abi::dev_flash::FLASH_MOUNT;

/// Workspace root, found by walking up to the manifest carrying
/// `[workspace]`. Integration tests run with the crate directory as
/// CWD, so relative corpus paths must anchor here.
pub fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        if std::fs::read_to_string(p.join("Cargo.toml")).is_ok_and(|t| t.contains("[workspace]")) {
            return p;
        }
        assert!(
            p.pop(),
            "workspace root not found above {}",
            env!("CARGO_MANIFEST_DIR")
        );
    }
}

/// The firmware version the corpus suites hold their assertions
/// against.
pub const CORPUS_FIRMWARE_VERSION: &str = "4.93";

/// Every installed firmware's `dev_flash` tree, keyed by version.
///
/// # Panics
///
/// Panics when:
///
/// - the firmware records directory cannot be read,
/// - a record cannot be parsed, or
/// - two records claim one version.
fn dev_flash_trees() -> BTreeMap<String, PathBuf> {
    let root = workspace_root().join(DEFAULT_VFS_ROOT);
    let layout = StoreLayout::new(&root);
    let records = layout.installs_dir().join(ArtifactKind::Firmware.as_str());
    let entries = std::fs::read_dir(&records).unwrap_or_else(|e| {
        panic!(
            "no firmware install records at {}: {e}. Run \
             `cellgov firmware install <PS3UPDAT.PUP>` to populate the store.",
            records.display()
        )
    });
    let mut found: BTreeMap<String, PathBuf> = BTreeMap::new();
    for entry in entries {
        // Skipping an unreadable entry would drop a firmware from the
        // census.
        let entry =
            entry.unwrap_or_else(|e| panic!("reading an entry of {}: {e}", records.display()));
        let path = entry.path();
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".install.toml"))
        {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let record = InstallRecord::parse(&text)
            .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()));
        let tree = layout
            .resolve_store_path(&record.artifact.store_path)
            .join(FLASH_MOUNT);
        if let Some(prev) = found.insert(record.artifact.version.clone(), tree) {
            panic!(
                "two firmware install records claim version {}: {} and {}",
                record.artifact.version,
                prev.display(),
                path.display()
            );
        }
    }
    found
}

/// The `dev_flash` tree of [`CORPUS_FIRMWARE_VERSION`].
///
/// # Panics
///
/// Panics when the store holds no entry for that version.
pub fn dev_flash() -> PathBuf {
    let trees = dev_flash_trees();
    trees
        .get(CORPUS_FIRMWARE_VERSION)
        .unwrap_or_else(|| {
            let installed: Vec<&str> = trees.keys().map(String::as_str).collect();
            panic!(
                "the corpus suites are held against firmware {CORPUS_FIRMWARE_VERSION}, \
                 which is not installed (installed: {}). Run \
                 `cellgov firmware install <PS3UPDAT.PUP>` for it.",
                installed.join(", ")
            )
        })
        .clone()
}

/// Firmware modules published to the guest.
///
/// # Panics
///
/// If the directory is absent. Every caller declares the corpus
/// installed, through the feature its own target is gated on or
/// through an `#[ignore]` opt-in.
pub fn firmware_external_dir() -> PathBuf {
    let dir = dev_flash().join("sys").join("external");
    assert!(
        dir.is_dir(),
        "no PS3 firmware corpus at {}. Run `cellgov firmware install <PS3UPDAT.PUP>` \
         to populate it.",
        dir.display()
    );
    dir
}
